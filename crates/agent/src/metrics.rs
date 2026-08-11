use chrono::Utc;
use pinglake_protocol::{DiskMetric, InterfaceMetric, MetricReport, ProcessMetric};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::time::{Duration, Instant};
use sysinfo::{Components, Disks, Networks, ProcessRefreshKind, ProcessesToUpdate, System};
const MAX_DETAIL_ITEMS: usize = 128;
const MAX_PROCESS_ITEMS: usize = 25;

#[derive(Debug)]
pub struct HostIdentity {
    pub hostname: String,
    pub display_name: String,
    pub os: String,
    pub os_version: String,
    pub kernel_version: String,
    pub architecture: String,
}

pub struct MetricCollector {
    system: System,
    disks: Disks,
    networks: Networks,
    components: Components,
    previous_network_totals: HashMap<String, NetworkTotals>,
    previous_network_time: Option<Instant>,
    last_hub_latency_ms: Option<f32>,
}

impl MetricCollector {
    pub fn new() -> Self {
        Self {
            // Process CPU percentages are deltas. Starting with an unrefreshed system gives the
            // first collection a baseline instead of treating startup work as process load.
            system: System::new(),
            disks: Disks::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            components: Components::new_with_refreshed_list(),
            previous_network_totals: HashMap::new(),
            previous_network_time: None,
            last_hub_latency_ms: None,
        }
    }

    pub fn host_identity(&self, configured_name: Option<&str>) -> HostIdentity {
        let hostname = System::host_name().unwrap_or_else(|| "unknown-host".to_owned());
        HostIdentity {
            display_name: configured_name.unwrap_or(&hostname).to_owned(),
            hostname,
            os: System::name().unwrap_or_else(|| std::env::consts::OS.to_owned()),
            os_version: detailed_os_version(),
            kernel_version: non_empty_system_value(System::kernel_version()),
            architecture: {
                let architecture = System::cpu_arch();
                if architecture.is_empty() {
                    std::env::consts::ARCH.to_owned()
                } else {
                    architecture
                }
            },
        }
    }

    pub fn collect(&mut self) -> MetricReport {
        self.system.refresh_memory();
        // `refresh_all` refreshes CPU before walking processes. On large Linux hosts, the walk
        // can cross sysinfo's refresh interval and trigger a second CPU refresh, producing a
        // tiny denominator for process CPU deltas. Refreshing processes owns the CPU update here.
        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
        self.disks.refresh(true);
        self.networks.refresh(true);
        self.components.refresh(true);

        let mut disks = self.collect_disks();
        let (disk_used_bytes, disk_total_bytes) = disk_totals(&disks);
        disks.truncate(MAX_DETAIL_ITEMS);
        let mut interfaces = self.collect_network_rates(Instant::now());
        let (network_received_bytes_per_sec, network_transmitted_bytes_per_sec) =
            interface_totals(&interfaces);
        interfaces.truncate(MAX_DETAIL_ITEMS);
        let load = load_average();
        let processes = self.collect_processes();

        MetricReport {
            collected_at: Utc::now(),
            cpu_percent: self.system.global_cpu_usage(),
            memory_used_bytes: self.system.used_memory(),
            memory_total_bytes: self.system.total_memory(),
            swap_used_bytes: self.system.used_swap(),
            swap_total_bytes: self.system.total_swap(),
            disk_used_bytes,
            disk_total_bytes,
            network_received_bytes_per_sec,
            network_transmitted_bytes_per_sec,
            hub_latency_ms: self.last_hub_latency_ms,
            load_one: load.as_ref().map(|value| value.one),
            load_five: load.as_ref().map(|value| value.five),
            load_fifteen: load.as_ref().map(|value| value.fifteen),
            temperature_celsius: highest_temperature(&self.components),
            uptime_seconds: System::uptime(),
            process_count: self.system.processes().len(),
            processes,
            disks,
            interfaces,
        }
    }

    pub fn record_hub_latency(&mut self, duration: Duration) {
        self.last_hub_latency_ms =
            Some((duration.as_secs_f64() * 1_000.0).clamp(0.0, 60_000.0) as f32);
    }

    fn collect_disks(&self) -> Vec<DiskMetric> {
        let mut metrics = self
            .disks
            .list()
            .iter()
            .filter(|disk| disk.total_space() > 0)
            .map(|disk| DiskMetric {
                name: os_string_lossy(disk.name()),
                mount_point: disk.mount_point().to_string_lossy().into_owned(),
                file_system: os_string_lossy(disk.file_system()),
                total_bytes: disk.total_space(),
                used_bytes: disk.total_space().saturating_sub(disk.available_space()),
            })
            .collect::<Vec<_>>();
        metrics.sort_by(|left, right| left.mount_point.cmp(&right.mount_point));
        metrics
    }

    fn collect_network_rates(&mut self, now: Instant) -> Vec<InterfaceMetric> {
        let elapsed = self
            .previous_network_time
            .map(|previous| now.saturating_duration_since(previous));
        let mut current_totals = HashMap::new();
        let mut metrics = Vec::with_capacity(self.networks.len());

        for (name, data) in &self.networks {
            let current = NetworkTotals {
                received: data.total_received(),
                transmitted: data.total_transmitted(),
            };
            let (received_rate, transmitted_rate) =
                match (self.previous_network_totals.get(name), elapsed) {
                    (Some(previous), Some(duration)) => (
                        bytes_per_second(previous.received, current.received, duration),
                        bytes_per_second(previous.transmitted, current.transmitted, duration),
                    ),
                    _ => (0, 0),
                };

            current_totals.insert(name.clone(), current);
            metrics.push(InterfaceMetric {
                name: name.clone(),
                received_bytes_per_sec: received_rate,
                transmitted_bytes_per_sec: transmitted_rate,
            });
        }

        metrics.sort_by(|left, right| left.name.cmp(&right.name));
        self.previous_network_totals = current_totals;
        self.previous_network_time = Some(now);
        metrics
    }

    fn collect_processes(&self) -> Vec<ProcessMetric> {
        let mut processes = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| ProcessMetric {
                pid: pid.as_u32(),
                name: os_string_lossy(process.name()),
                cpu_percent: process.cpu_usage(),
                memory_bytes: process.memory(),
            })
            .collect::<Vec<_>>();
        processes.sort_by(|left, right| {
            right
                .cpu_percent
                .total_cmp(&left.cpu_percent)
                .then_with(|| right.memory_bytes.cmp(&left.memory_bytes))
                .then_with(|| left.pid.cmp(&right.pid))
        });
        processes.truncate(MAX_PROCESS_ITEMS);
        processes
    }
}

#[derive(Clone, Copy)]
struct NetworkTotals {
    received: u64,
    transmitted: u64,
}

fn bytes_per_second(previous: u64, current: u64, elapsed: Duration) -> u64 {
    if current < previous || elapsed.is_zero() {
        return 0;
    }
    let elapsed_seconds = elapsed.as_secs_f64();
    if elapsed_seconds <= 0.0 {
        return 0;
    }
    ((current - previous) as f64 / elapsed_seconds)
        .round()
        .clamp(0.0, u64::MAX as f64) as u64
}

fn disk_totals(disks: &[DiskMetric]) -> (u64, u64) {
    disks.iter().fold((0_u64, 0_u64), |(used, total), disk| {
        (
            used.saturating_add(disk.used_bytes.min(disk.total_bytes)),
            total.saturating_add(disk.total_bytes),
        )
    })
}

fn interface_totals(interfaces: &[InterfaceMetric]) -> (u64, u64) {
    interfaces
        .iter()
        .fold((0_u64, 0_u64), |(received, transmitted), interface| {
            (
                received.saturating_add(interface.received_bytes_per_sec),
                transmitted.saturating_add(interface.transmitted_bytes_per_sec),
            )
        })
}

fn highest_temperature(components: &Components) -> Option<f32> {
    components
        .list()
        .iter()
        .filter_map(|component| component.temperature())
        .filter(|temperature| temperature.is_finite())
        .max_by(f32::total_cmp)
}

#[cfg(unix)]
fn load_average() -> Option<sysinfo::LoadAvg> {
    Some(System::load_average())
}

#[cfg(not(unix))]
fn load_average() -> Option<sysinfo::LoadAvg> {
    None
}

fn os_string_lossy(value: &OsStr) -> String {
    value.to_string_lossy().into_owned()
}

fn non_empty_system_value(value: Option<String>) -> String {
    value
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(target_os = "linux")]
fn detailed_os_version() -> String {
    let pretty_name = std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("PRETTY_NAME=").map(|value| {
                    value
                        .trim_matches('"')
                        .replace(r#"\""#, "\"")
                        .replace(r#"\\"#, "\\")
                })
            })
        });
    pretty_name.unwrap_or_else(|| non_empty_system_value(System::os_version()))
}

#[cfg(target_os = "windows")]
fn detailed_os_version() -> String {
    let raw_product = System::long_os_version().unwrap_or_else(|| "Windows".to_owned());
    let product = raw_product
        .split(|character| matches!(character, '\u{00b7}' | '\u{00c2}'))
        .next()
        .unwrap_or("Windows")
        .trim()
        .to_owned();
    let build = System::kernel_version()
        .or_else(System::os_version)
        .and_then(|version| {
            version
                .split(|character: char| !character.is_ascii_digit())
                .filter(|component| component.len() >= 3)
                .next_back()
                .map(str::to_owned)
        });

    build
        .map(|value| format!("{product} - build {value}"))
        .unwrap_or(product)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn detailed_os_version() -> String {
    non_empty_system_value(System::long_os_version().or_else(System::os_version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_network_rate_from_counter_delta() {
        assert_eq!(
            bytes_per_second(1_000, 4_000, Duration::from_millis(1_500)),
            2_000
        );
    }

    #[test]
    fn network_counter_reset_does_not_create_a_spike() {
        assert_eq!(bytes_per_second(4_000, 1_000, Duration::from_secs(1)), 0);
        assert_eq!(bytes_per_second(0, 10, Duration::ZERO), 0);
    }

    #[test]
    fn disk_summary_is_saturating_and_caps_invalid_used_values() {
        let disks = vec![
            DiskMetric {
                name: "one".into(),
                mount_point: "/one".into(),
                file_system: "test".into(),
                total_bytes: 100,
                used_bytes: 25,
            },
            DiskMetric {
                name: "two".into(),
                mount_point: "/two".into(),
                file_system: "test".into(),
                total_bytes: 200,
                used_bytes: 250,
            },
        ];

        assert_eq!(disk_totals(&disks), (225, 300));
    }

    #[test]
    fn interface_summary_saturates() {
        let interfaces = vec![
            InterfaceMetric {
                name: "one".into(),
                received_bytes_per_sec: u64::MAX,
                transmitted_bytes_per_sec: 10,
            },
            InterfaceMetric {
                name: "two".into(),
                received_bytes_per_sec: 10,
                transmitted_bytes_per_sec: 20,
            },
        ];

        assert_eq!(interface_totals(&interfaces), (u64::MAX, 30));
    }

    #[test]
    fn first_process_sample_uses_a_baseline() {
        let mut collector = MetricCollector::new();

        let report = collector.collect();

        assert!(
            report
                .processes
                .iter()
                .all(|process| process.cpu_percent == 0.0)
        );
    }
}
