use std::collections::BTreeMap;
use std::net::IpAddr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MONITORING_SCHEMA_V1: u32 = 1;
pub const MONITORING_SCHEMA_V2: u32 = 2;
pub const MAX_CHECKS_PER_NODE: usize = 32;
pub const MAX_DNS_ANSWERS: usize = 64;
pub const MAX_CHECK_ERROR_BYTES: usize = 1024;

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

/// Collection state is distinct from whether the observed target met its
/// expectation. Results use `healthy: Option<bool>` for that second question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    WarmingUp,
    Unsupported,
    PermissionDenied,
    #[default]
    Unavailable,
    Stale,
    PolicyDenied,
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
    /// Transport metadata; excluded from canonical report content bytes.
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_checks: Vec<ProcessResult>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_port_checks: Vec<LocalPortResult>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsRecordType {
    #[serde(rename = "A")]
    A,
    #[serde(rename = "AAAA")]
    Aaaa,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsProbeOptions {
    pub record_type: DnsRecordType,
    /// Exact normalized answer equality. This is not substring, regex, or
    /// suffix matching. Without an expected value, a valid answer is enough.
    pub expected_value: Option<String>,
}

impl DnsProbeOptions {
    pub fn expected_matches(&self, answers: &[String]) -> bool {
        match self.expected_value.as_deref() {
            Some(expected) => answers.iter().any(|answer| {
                canonical_dns_answer(self.record_type, answer)
                    == canonical_dns_answer(self.record_type, expected)
            }),
            None => !answers.is_empty(),
        }
    }
}

fn canonical_dns_answer(record_type: DnsRecordType, value: &str) -> Option<String> {
    match record_type {
        DnsRecordType::A | DnsRecordType::Aaaa => {
            value.parse::<IpAddr>().ok().map(|ip| ip.to_string())
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsObservation {
    pub record_type: DnsRecordType,
    /// DNS wire RCODE. None means no DNS response was received.
    pub rcode: Option<u16>,
    pub answers: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    #[serde(rename = "icmp")]
    #[default]
    Icmp,
    Tcp,
    Http,
    Dns,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<DnsProbeOptions>,
}

impl Default for ProbeTarget {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            kind: ProbeKind::Icmp,
            target: String::new(),
            port: None,
            enabled: true,
            interval_secs: interval(),
            timeout_ms: timeout(),
            expected_status: None,
            response_contains: None,
            dns: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalPortProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalPortAddressFamily {
    Any,
    Ipv4,
    Ipv6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum LocalPortAddressScope {
    AnyLocal,
    Loopback,
    Exact { address: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessCheck {
    pub id: Uuid,
    pub name: String,
    pub process_name: String,
    pub expected_count: Option<u32>,
    pub enabled: bool,
    pub expected_state: String,
    #[serde(default = "interval")]
    pub interval_secs: u64,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
}

impl ProcessCheck {
    pub fn expects_running(&self) -> bool {
        self.expected_state == "running"
    }
}

impl Default for ProcessCheck {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            process_name: String::new(),
            expected_count: None,
            enabled: true,
            expected_state: running(),
            interval_secs: interval(),
            timeout_ms: timeout(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPortCheck {
    pub id: Uuid,
    pub name: String,
    /// Optional only for legacy stored JSON; v2 wire configs must set it.
    pub address_scope: Option<LocalPortAddressScope>,
    /// Optional only for legacy stored JSON; v2 wire configs must set it.
    pub address_family: Option<LocalPortAddressFamily>,
    /// Optional only for legacy stored JSON; v2 wire configs must set it.
    pub protocol: Option<LocalPortProtocol>,
    pub port: u16,
    pub enabled: bool,
    #[serde(default = "interval")]
    pub interval_secs: u64,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
}

impl Default for LocalPortCheck {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            address_scope: None,
            address_family: None,
            protocol: None,
            port: 0,
            enabled: true,
            interval_secs: interval(),
            timeout_ms: timeout(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodeMonitoringConfig {
    pub revision: u64,
    pub browser_latency_url: Option<String>,
    pub services: Vec<ServiceCheck>,
    pub probes: Vec<ProbeTarget>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub process_checks: Vec<ProcessCheck>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_port_checks: Vec<LocalPortCheck>,
}

impl NodeMonitoringConfig {
    /// v1 contains only service and ICMP/TCP/HTTP fields. New fields are
    /// omitted from the wire object, rather than merely sent as empty arrays.
    pub fn to_wire_json(&self, monitoring_schema_max: u32) -> serde_json::Result<Vec<u8>> {
        let mut value = serde_json::to_value(self)?;
        if monitoring_schema_max < MONITORING_SCHEMA_V2 {
            downgrade_config_value(&mut value);
        }
        serde_json::to_vec(&value)
    }
}

fn downgrade_config_value(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.remove("process_checks");
    object.remove("local_port_checks");
    let Some(probes) = object
        .get_mut("probes")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    probes.retain(|probe| probe.get("kind").and_then(serde_json::Value::as_str) != Some("dns"));
    for probe in probes {
        if let Some(probe) = probe.as_object_mut() {
            probe.remove("dns");
        }
    }
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
    #[serde(default)]
    pub healthy: Option<bool>,
    pub latency_ms: Option<f64>,
    pub http_status: Option<u16>,
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns: Option<DnsObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessResult {
    pub id: Uuid,
    pub name: String,
    pub sample_id: Option<Uuid>,
    pub config_revision: Option<u64>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub checked_at: Option<DateTime<Utc>>,
    pub status: CheckStatus,
    pub healthy: Option<bool>,
    pub process_name: String,
    pub count: Option<u32>,
    pub expected_count: Option<u32>,
    pub error: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalPortResult {
    pub id: Uuid,
    pub name: String,
    pub sample_id: Option<Uuid>,
    pub config_revision: Option<u64>,
    pub scheduled_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub checked_at: Option<DateTime<Utc>>,
    pub status: CheckStatus,
    pub healthy: Option<bool>,
    pub address_scope: Option<LocalPortAddressScope>,
    pub address_family: Option<LocalPortAddressFamily>,
    pub protocol: Option<LocalPortProtocol>,
    pub port: u16,
    pub observed_addresses: Vec<String>,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
    pub reason: Option<String>,
}

impl Default for ProcessResult {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            sample_id: None,
            config_revision: None,
            scheduled_at: None,
            completed_at: None,
            checked_at: None,
            status: CheckStatus::Unavailable,
            healthy: None,
            process_name: String::new(),
            count: None,
            expected_count: None,
            error: None,
            reason: None,
        }
    }
}

impl Default for LocalPortResult {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            sample_id: None,
            config_revision: None,
            scheduled_at: None,
            completed_at: None,
            checked_at: None,
            status: CheckStatus::Unavailable,
            healthy: None,
            address_scope: None,
            address_family: None,
            protocol: None,
            port: 0,
            observed_addresses: Vec::new(),
            latency_ms: None,
            error: None,
            reason: None,
        }
    }
}

pub type PortCheck = LocalPortCheck;
pub type PortResult = LocalPortResult;

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
    use crate::MetricReport;

    fn legacy_report() -> MetricReport {
        serde_json::from_value(serde_json::json!({
            "collected_at": "2026-10-01T00:00:00Z", "cpu_percent": 12.0,
            "memory_used_bytes": 100, "memory_total_bytes": 200, "swap_used_bytes": 0,
            "swap_total_bytes": 0, "disk_used_bytes": 20, "disk_total_bytes": 100,
            "network_received_bytes_per_sec": 0, "network_transmitted_bytes_per_sec": 0,
            "load_one": null, "load_five": null, "load_fifteen": null,
            "temperature_celsius": null, "uptime_seconds": 5, "process_count": 1,
            "disks": [], "interfaces": []
        }))
        .unwrap()
    }

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
        assert!(serde_json::from_str::<MonitoringData>("{}").is_ok());
    }

    #[test]
    fn legacy_reports_and_configs_remain_readable_without_fabricating_ids() {
        assert!(legacy_report().monitoring.is_none());
        let config: NodeMonitoringConfig = serde_json::from_value(serde_json::json!({
            "revision": 1, "browser_latency_url": null,
            "services": [{"id":"00000000-0000-0000-0000-000000000001","name":"EventLog"}],
            "probes": [{"id":"00000000-0000-0000-0000-000000000002","name":"http","kind":"http","target":"https://example.test/ping","port":null,"enabled":true,"interval_secs":30,"timeout_ms":5000,"expected_status":null,"response_contains":null}]
        })).unwrap();
        assert!(config.process_checks.is_empty());
        let old_process: ProcessResult = serde_json::from_str(r#"{"id":"00000000-0000-0000-0000-000000000001","name":"worker","status":"unavailable","process_name":"worker"}"#).unwrap();
        assert!(old_process.sample_id.is_none());
        assert!(
            serde_json::from_value::<ProbeTarget>(
                serde_json::json!({"name":"missing-id","kind":"icmp","target":"127.0.0.1"})
            )
            .is_err()
        );
    }

    #[test]
    fn dns_is_one_typed_probe_channel_with_exact_answer_matching() {
        let options = DnsProbeOptions {
            record_type: DnsRecordType::A,
            expected_value: Some("192.0.2.1".into()),
        };
        assert!(options.expected_matches(&["192.0.2.1".into()]));
        assert!(!options.expected_matches(&["192.0.2.10".into()]));
        assert!(serde_json::from_value::<NodeMonitoringConfig>(serde_json::json!({
            "revision": 2, "browser_latency_url": null, "services": [],
            "probes": [{"id":"00000000-0000-0000-0000-000000000010","name":"dns","kind":"dns","target":"example.test","port":null,"enabled":true,"interval_secs":30,"timeout_ms":5000,"expected_status":null,"response_contains":null,"dns":{"record_type":"A","expected_value":"192.0.2.1"}}]
        })).is_ok());
        assert!(
            serde_json::from_value::<DnsObservation>(
                serde_json::json!({"record_type":"AAAA","rcode":3,"answers":[]})
            )
            .is_ok()
        );
    }

    #[test]
    fn process_and_port_results_separate_collection_status_from_health() {
        let process: ProcessResult = serde_json::from_value(serde_json::json!({
            "id":"00000000-0000-0000-0000-000000000020", "name":"worker",
            "sample_id":"00000000-0000-0000-0000-000000000021", "config_revision":4,
            "scheduled_at":"2026-10-01T00:00:00Z", "completed_at":"2026-10-01T00:00:01Z",
            "status":"ok", "healthy":false, "process_name":"worker", "count":0, "expected_count":1
        }))
        .unwrap();
        assert_eq!(process.status, CheckStatus::Ok);
        assert_eq!(process.healthy, Some(false));
        let unavailable: LocalPortResult = serde_json::from_value(serde_json::json!({
            "id":"00000000-0000-0000-0000-000000000022", "name":"http", "status":"permission_denied", "port":8080, "observed_addresses":[]
        })).unwrap();
        assert_eq!(unavailable.healthy, None);
        assert!(unavailable.observed_addresses.is_empty());
    }

    #[test]
    fn v1_wire_config_omits_unknown_fields_and_unsupported_dns_probe() {
        let config = NodeMonitoringConfig {
            revision: 4,
            services: vec![ServiceCheck {
                id: Uuid::new_v4(),
                name: "EventLog".into(),
                ..Default::default()
            }],
            probes: vec![
                ProbeTarget {
                    id: Uuid::new_v4(),
                    name: "http".into(),
                    kind: ProbeKind::Http,
                    target: "https://example.test".into(),
                    ..Default::default()
                },
                ProbeTarget {
                    id: Uuid::new_v4(),
                    name: "dns".into(),
                    kind: ProbeKind::Dns,
                    target: "example.test".into(),
                    dns: Some(DnsProbeOptions {
                        record_type: DnsRecordType::A,
                        expected_value: None,
                    }),
                    ..Default::default()
                },
            ],
            process_checks: vec![ProcessCheck {
                id: Uuid::new_v4(),
                name: "worker".into(),
                process_name: "worker".into(),
                ..Default::default()
            }],
            local_port_checks: vec![LocalPortCheck {
                id: Uuid::new_v4(),
                name: "http".into(),
                port: 8080,
                ..Default::default()
            }],
            ..Default::default()
        };
        let wire: serde_json::Value =
            serde_json::from_slice(&config.to_wire_json(MONITORING_SCHEMA_V1).unwrap()).unwrap();
        assert!(wire.get("process_checks").is_none());
        assert!(wire.get("local_port_checks").is_none());
        assert_eq!(wire["probes"].as_array().unwrap().len(), 1);
        assert!(wire["probes"][0].get("dns").is_none());
    }

    #[test]
    fn v2_fixture_uses_probe_dns_observation_and_fixed_execution_identity() {
        let data: MonitoringData =
            serde_json::from_str(include_str!("../fixtures/monitoring-v2.json")).unwrap();
        assert_eq!(data.schema_version, MONITORING_SCHEMA_V2);
        let dns = data
            .probes
            .iter()
            .find(|probe| probe.kind == ProbeKind::Dns)
            .unwrap();
        assert_eq!(dns.dns.as_ref().unwrap().rcode, Some(0));
        assert!(data.process_checks[0].sample_id.is_some());
        assert_eq!(
            data.local_port_checks[0].protocol,
            Some(LocalPortProtocol::Tcp)
        );
    }
}
