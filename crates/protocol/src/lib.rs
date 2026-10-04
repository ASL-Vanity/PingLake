#![recursion_limit = "256"]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod monitoring;
pub use monitoring::*;

pub const API_VERSION: &str = "v1";
pub const DEFAULT_REPORT_INTERVAL_SECS: u64 = 5;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollRequest {
    pub agent_id: Uuid,
    pub agent_secret: String,
    pub hostname: String,
    pub display_name: String,
    pub os: String,
    pub os_version: String,
    pub kernel_version: String,
    pub architecture: String,
    pub agent_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrollResponse {
    pub accepted: bool,
    pub report_interval_secs: u64,
    /// The highest monitoring report schema understood by this Hub. Older
    /// Hubs omit the field, so new Agents must treat the default as v1.
    #[serde(default = "default_monitoring_schema_max")]
    pub monitoring_schema_max: u32,
}

fn default_monitoring_schema_max() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskMetric {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub used_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InterfaceMetric {
    pub name: String,
    pub received_bytes_per_sec: u64,
    pub transmitted_bytes_per_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessMetric {
    pub pid: u32,
    pub name: String,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetricReport {
    pub collected_at: DateTime<Utc>,
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub swap_used_bytes: u64,
    pub swap_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub network_received_bytes_per_sec: u64,
    pub network_transmitted_bytes_per_sec: u64,
    #[serde(default)]
    pub hub_latency_ms: Option<f32>,
    pub load_one: Option<f64>,
    pub load_five: Option<f64>,
    pub load_fifteen: Option<f64>,
    pub temperature_celsius: Option<f32>,
    pub uptime_seconds: u64,
    pub process_count: usize,
    #[serde(default)]
    pub processes: Vec<ProcessMetric>,
    pub disks: Vec<DiskMetric>,
    pub interfaces: Vec<InterfaceMetric>,
    #[serde(default)]
    pub monitoring: Option<MonitoringData>,
}

impl MetricReport {
    /// Serialize a report for the negotiated Hub schema. A v1 receiver gets
    /// the original resource/service/ICMP/TCP/HTTP report shape; v2-only
    /// process, local-port and DNS fields are omitted from the wire object.
    /// New Agents must use this for retries and downgrade replays.
    pub fn to_wire_json(&self, monitoring_schema_max: u32) -> serde_json::Result<Vec<u8>> {
        let mut value = serde_json::to_value(self)?;
        if monitoring_schema_max < MONITORING_SCHEMA_V2 {
            downgrade_report_value(&mut value);
        }
        serde_json::to_vec(&value)
    }

    /// Return canonical content bytes for the Hub's same-identity conflict
    /// check. Identity envelope fields and all Agent transport health fields,
    /// including `sample_age_ms`, are excluded. The Hub may hash these bytes
    /// with SHA-256; retransmission metadata therefore cannot create a false
    /// payload conflict.
    pub fn canonical_content_bytes(&self) -> serde_json::Result<Vec<u8>> {
        let mut value = serde_json::to_value(self)?;
        if let Some(monitoring) = value
            .get_mut("monitoring")
            .and_then(serde_json::Value::as_object_mut)
        {
            for key in [
                "schema_version",
                "session_id",
                "sample_sequence",
                "report_interval_secs",
                "agent",
            ] {
                monitoring.remove(key);
            }
        }
        serde_json::to_vec(&value)
    }
}

fn downgrade_report_value(value: &mut serde_json::Value) {
    let Some(object) = value.as_object_mut() else {
        return;
    };
    let Some(monitoring) = object
        .get_mut("monitoring")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return;
    };
    monitoring.insert(
        "schema_version".into(),
        serde_json::Value::from(MONITORING_SCHEMA_V1),
    );
    monitoring.remove("process_checks");
    monitoring.remove("local_port_checks");
    let Some(probes) = monitoring
        .get_mut("probes")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return;
    };
    probes.retain(|probe| probe.get("kind").and_then(serde_json::Value::as_str) != Some("dns"));
    for probe in probes {
        let Some(probe) = probe.as_object_mut() else {
            continue;
        };
        probe.remove("dns");
        if let Some(status) = probe
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        {
            let downgraded = match status.as_str() {
                "warming_up" | "stale" => "unsupported",
                "unavailable" => "failure",
                _ => status.as_str(),
            };
            probe.insert(
                "status".into(),
                serde_json::Value::String(downgraded.into()),
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSnapshot {
    pub id: Uuid,
    pub hostname: String,
    pub display_name: String,
    pub os: String,
    pub os_version: String,
    pub kernel_version: String,
    pub architecture: String,
    pub agent_version: String,
    pub group_id: Option<Uuid>,
    pub group_name: Option<String>,
    pub enrolled_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub online: bool,
    pub latest: Option<MetricReport>,
    #[serde(default)]
    pub browser_latency_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostGroup {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryPoint {
    pub collected_at: DateTime<Utc>,
    pub cpu_percent: f32,
    pub memory_used_bytes: u64,
    pub memory_total_bytes: u64,
    pub disk_used_bytes: u64,
    pub disk_total_bytes: u64,
    pub network_received_bytes_per_sec: u64,
    pub network_transmitted_bytes_per_sec: u64,
    #[serde(default)]
    pub hub_latency_ms: Option<f32>,
    pub temperature_celsius: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertKind {
    Offline,
    Cpu,
    Memory,
    Disk,
    Temperature,
    Service,
    Probe,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRecord {
    pub id: i64,
    pub node_id: Uuid,
    pub node_name: String,
    pub kind: AlertKind,
    pub message: String,
    pub value: Option<f64>,
    pub threshold: Option<f64>,
    pub active: bool,
    pub opened_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub subject_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertSettings {
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub disk_percent: f64,
    pub temperature_celsius: f64,
    pub offline_after_seconds: u64,
    pub sustained_for_seconds: u64,
    #[serde(default = "default_true")]
    pub offline_enabled: bool,
    #[serde(default = "default_true")]
    pub cpu_enabled: bool,
    #[serde(default = "default_true")]
    pub memory_enabled: bool,
    #[serde(default = "default_true")]
    pub disk_enabled: bool,
    #[serde(default = "default_true")]
    pub temperature_enabled: bool,
    pub webhook_enabled: bool,
    pub webhook_url: String,
    #[serde(default)]
    pub email_enabled: bool,
    #[serde(default)]
    pub email_recipients: Vec<String>,
}

fn default_true() -> bool {
    true
}

impl Default for AlertSettings {
    fn default() -> Self {
        Self {
            cpu_percent: 85.0,
            memory_percent: 90.0,
            disk_percent: 85.0,
            temperature_celsius: 85.0,
            offline_after_seconds: 20,
            sustained_for_seconds: 60,
            offline_enabled: true,
            cpu_enabled: true,
            memory_enabled: true,
            disk_enabled: true,
            temperature_enabled: true,
            webhook_enabled: false,
            webhook_url: String::new(),
            email_enabled: false,
            email_recipients: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummary {
    pub total_nodes: usize,
    pub online_nodes: usize,
    pub offline_nodes: usize,
    pub active_alerts: usize,
    pub average_cpu_percent: f64,
    pub average_memory_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum LiveEvent {
    Snapshot(Box<NodeSnapshot>),
    Alert(AlertRecord),
    NodeRemoved { id: Uuid },
    SettingsChanged(AlertSettings),
    GroupsChanged(Vec<HostGroup>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

#[cfg(test)]
mod tests {
    use super::{
        AgentHealth, EnrollResponse, MONITORING_SCHEMA_V1, MONITORING_SCHEMA_V2, MetricReport,
        MonitoringData, ProbeResult, ProbeStatus, Uuid,
    };

    #[test]
    fn legacy_enrollment_response_defaults_to_v1_monitoring() {
        let response: EnrollResponse =
            serde_json::from_str(r#"{"accepted":true,"report_interval_secs":5}"#).unwrap();
        assert_eq!(response.monitoring_schema_max, 1);
    }

    #[test]
    fn v1_report_wire_downgrade_omits_v2_results_and_dns() {
        let report: MetricReport = serde_json::from_value(serde_json::json!({
            "collected_at":"2026-10-01T00:00:00Z","cpu_percent":1.0,
            "memory_used_bytes":1,"memory_total_bytes":2,"swap_used_bytes":0,"swap_total_bytes":0,
            "disk_used_bytes":0,"disk_total_bytes":1,"network_received_bytes_per_sec":0,
            "network_transmitted_bytes_per_sec":0,"load_one":null,"load_five":null,"load_fifteen":null,
            "temperature_celsius":null,"uptime_seconds":1,"process_count":0,"disks":[],"interfaces":[],
            "monitoring": {"schema_version":2,"session_id":"00000000-0000-0000-0000-000000000001",
              "sample_sequence":1,"report_interval_secs":30,"capabilities":{},"cpu_cores":[],
              "cpu_times":{},"memory":{},"disk_io":[],"inodes":[],"network_health":[],"tcp":{},"agent":{},
              "services":[],"process_checks":[{"id":"00000000-0000-0000-0000-000000000002","name":"p","status":"ok","process_name":"p"}],
              "local_port_checks":[],"probes":[{"sample_id":"00000000-0000-0000-0000-000000000003","target_id":"00000000-0000-0000-0000-000000000004","config_revision":1,"kind":"dns","scheduled_at":"2026-10-01T00:00:00Z","completed_at":"2026-10-01T00:00:00Z","status":"failure","healthy":false,"latency_ms":null,"http_status":null,"error":"dns","dns":{"record_type":"A","rcode":3,"answers":[]}}]
            }
        })).unwrap();
        let wire: serde_json::Value =
            serde_json::from_slice(&report.to_wire_json(MONITORING_SCHEMA_V1).unwrap()).unwrap();
        let monitoring = &wire["monitoring"];
        assert_eq!(monitoring["schema_version"], MONITORING_SCHEMA_V1);
        assert!(monitoring.get("process_checks").is_none());
        assert!(monitoring["probes"].as_array().unwrap().is_empty());
    }

    #[test]
    fn canonical_content_excludes_agent_transport_metadata() {
        let mut report = MetricReport {
            collected_at: chrono::Utc::now(),
            cpu_percent: 1.0,
            memory_used_bytes: 0,
            memory_total_bytes: 1,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            disk_used_bytes: 0,
            disk_total_bytes: 1,
            network_received_bytes_per_sec: 0,
            network_transmitted_bytes_per_sec: 0,
            hub_latency_ms: None,
            load_one: None,
            load_five: None,
            load_fifteen: None,
            temperature_celsius: None,
            uptime_seconds: 1,
            process_count: 0,
            processes: vec![],
            disks: vec![],
            interfaces: vec![],
            monitoring: Some(MonitoringData {
                schema_version: MONITORING_SCHEMA_V2,
                session_id: Uuid::new_v4(),
                sample_sequence: 1,
                agent: AgentHealth {
                    sample_age_ms: 1.0,
                    upload_attempts: 1,
                    ..Default::default()
                },
                ..Default::default()
            }),
        };
        let first = report.canonical_content_bytes().unwrap();
        report.monitoring.as_mut().unwrap().agent.sample_age_ms = 9999.0;
        report.monitoring.as_mut().unwrap().sample_sequence = 2;
        assert_eq!(first, report.canonical_content_bytes().unwrap());
        report.cpu_percent = 2.0;
        assert_ne!(first, report.canonical_content_bytes().unwrap());
    }

    #[test]
    fn v2_probe_keeps_dns_observation_optional_for_old_normal_reports() {
        let result: ProbeResult = serde_json::from_value(serde_json::json!({
            "sample_id":"00000000-0000-0000-0000-000000000003","target_id":"00000000-0000-0000-0000-000000000004",
            "config_revision":1,"kind":"http","scheduled_at":"2026-10-01T00:00:00Z","completed_at":"2026-10-01T00:00:00Z",
            "status":"success","latency_ms":1.0,"http_status":200,"error":null
        })).unwrap();
        assert!(result.dns.is_none());
        assert_eq!(ProbeStatus::Success, ProbeStatus::Success);
        assert_eq!(MONITORING_SCHEMA_V2, 2);
    }
}
