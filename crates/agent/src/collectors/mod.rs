use pinglake_protocol::{Capability, CpuCore, DiskMetric, MetricStatus, MonitoringData};
use std::time::Instant;
use sysinfo::System;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(target_os = "linux")]
use linux::PlatformCollector;
#[cfg(target_os = "windows")]
use windows::PlatformCollector;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
struct PlatformCollector;

pub struct DetailedCollector {
    platform: PlatformCollector,
    started: bool,
}

impl DetailedCollector {
    pub fn new() -> Self {
        Self {
            platform: PlatformCollector::new(),
            started: false,
        }
    }

    pub fn collect(&mut self, system: &System, disks: &[DiskMetric]) -> MonitoringData {
        let mut data = MonitoringData {
            schema_version: 1,
            ..Default::default()
        };
        let cpu_status = if self.started {
            MetricStatus::Ok
        } else {
            MetricStatus::WarmingUp
        };
        data.capabilities
            .insert("cpu_cores".into(), capability(cpu_status, "sysinfo"));
        data.capabilities
            .insert("memory".into(), capability(MetricStatus::Ok, "sysinfo"));
        data.cpu_cores = system
            .cpus()
            .iter()
            .enumerate()
            .take(1024)
            .map(|(index, cpu)| CpuCore {
                id: index.to_string(),
                usage_percent: cpu.cpu_usage(),
                frequency_mhz: (cpu.frequency() > 0).then(|| cpu.frequency()),
            })
            .collect();
        data.memory.available_bytes = Some(system.available_memory());
        self.platform.collect(Instant::now(), disks, &mut data);
        self.started = true;
        data
    }
}

pub(super) fn capability(status: MetricStatus, source: &str) -> Capability {
    Capability {
        status,
        source: source.into(),
        error: None,
    }
}

pub(super) fn rate(previous: u64, current: u64, elapsed: f64) -> Option<f64> {
    (current >= previous && elapsed.is_finite() && elapsed > 0.0)
        .then(|| (current - previous) as f64 / elapsed)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl PlatformCollector {
    fn new() -> Self {
        Self
    }
    fn collect(&mut self, _: Instant, _: &[DiskMetric], data: &mut MonitoringData) {
        for name in [
            "cpu_times",
            "swap_io",
            "disk_io",
            "inodes",
            "network_health",
            "tcp",
        ] {
            data.capabilities.insert(
                name.into(),
                capability(MetricStatus::Unsupported, "platform"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_handles_reset_and_invalid_interval() {
        assert_eq!(rate(20, 30, 2.0), Some(5.0));
        assert_eq!(rate(30, 20, 2.0), None);
        assert_eq!(rate(0, 1, 0.0), None);
    }
}
