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
