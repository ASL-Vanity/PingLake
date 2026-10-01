use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::CString;
use std::fs;
use std::io;
use std::time::Instant;

use super::{capability, rate};
use pinglake_protocol::{
    CpuTimes, DiskIo, DiskMetric, InodeMetric, MetricStatus, MonitoringData, NetworkHealth,
    TcpMetrics,
};

#[derive(Clone, Debug)]
struct DiskCounters {
    id: String,
    name: String,
    values: [u64; 11],
}
#[derive(Clone, Debug)]
struct NetworkCounters {
    name: String,
    values: [u64; 16],
}

pub struct PlatformCollector {
    previous_cpu: Option<[u64; 8]>,
    previous_swap: Option<(u64, u64, Instant)>,
    previous_disks: HashMap<String, (DiskCounters, Instant)>,
    previous_network: HashMap<String, (NetworkCounters, Instant)>,
}

impl PlatformCollector {
    pub fn new() -> Self {
        Self {
            previous_cpu: None,
            previous_swap: None,
            previous_disks: HashMap::new(),
            previous_network: HashMap::new(),
        }
    }

    pub fn collect(&mut self, now: Instant, disks: &[DiskMetric], data: &mut MonitoringData) {
        self.cpu(data);
        self.memory(now, data);
        self.disks(now, data);
        self.network(now, data);
        data.inodes = disks.iter().map(inodes).collect();
        let inode_status = if data.inodes.iter().any(|v| v.status == MetricStatus::Ok) {
            MetricStatus::Ok
        } else {
            data.inodes
                .first()
                .map(|v| v.status.clone())
                .unwrap_or(MetricStatus::Unavailable)
        };
        data.capabilities
            .insert("inodes".into(), capability(inode_status, "statvfs"));
        self.tcp(data);
    }

    fn cpu(&mut self, data: &mut MonitoringData) {
        let current = fs::read_to_string("/proc/stat").and_then(|text| parse_cpu(&text));
        match current {
            Ok(current) => {
                let times = self
                    .previous_cpu
                    .and_then(|previous| cpu_delta(previous, current));
                data.capabilities.insert(
                    "cpu_times".into(),
                    capability(
                        if times.is_some() {
                            MetricStatus::Ok
                        } else {
                            MetricStatus::WarmingUp
                        },
                        "/proc/stat",
                    ),
                );
                data.cpu_times = times.unwrap_or_default();
                self.previous_cpu = Some(current);
            }
            Err(error) => {
                self.previous_cpu = None;
                failed(data, "cpu_times", "/proc/stat", &error);
            }
        }
    }

    fn memory(&mut self, now: Instant, data: &mut MonitoringData) {
        match fs::read_to_string("/proc/meminfo").and_then(|text| parse_meminfo(&text)) {
            Ok(values) => {
                data.capabilities.insert(
                    "memory_details".into(),
                    capability(MetricStatus::Ok, "/proc/meminfo"),
                );
                data.memory.cached_bytes = values.get("Cached").copied();
                data.memory.buffers_bytes = values.get("Buffers").copied();
                // The kernel's MemAvailable includes reclaimable memory and differs from free RAM.
                if let Some(value) = values.get("MemAvailable") {
                    data.memory.available_bytes = Some(*value);
                }
            }
            Err(error) => failed(data, "memory_details", "/proc/meminfo", &error),
        }
        let counters = fs::read_to_string("/proc/vmstat").and_then(|text| parse_swap(&text));
        match counters {
            Ok((pages_in, pages_out)) => {
                let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
                let counts = (page_size > 0)
                    .then(|| {
                        (
                            pages_in.checked_mul(page_size as u64),
                            pages_out.checked_mul(page_size as u64),
                        )
                    })
                    .and_then(|(input, output)| input.zip(output));
                let Some((input, output)) = counts else {
                    failed(
                        data,
                        "swap_io",
                        "/proc/vmstat",
                        &invalid("invalid swap counter or page size"),
                    );
                    self.previous_swap = None;
                    return;
                };
                data.memory.swap_in_bytes = Some(input.to_string());
                data.memory.swap_out_bytes = Some(output.to_string());
                if let Some((previous_in, previous_out, at)) = self.previous_swap {
                    let elapsed = now.duration_since(at).as_secs_f64();
                    data.memory.swap_in_bytes_per_sec = rate(previous_in, input, elapsed);
                    data.memory.swap_out_bytes_per_sec = rate(previous_out, output, elapsed);
                }
                let status = if data.memory.swap_in_bytes_per_sec.is_some()
                    && data.memory.swap_out_bytes_per_sec.is_some()
                {
                    MetricStatus::Ok
                } else {
                    MetricStatus::WarmingUp
                };
                data.capabilities
                    .insert("swap_io".into(), capability(status, "/proc/vmstat"));
                self.previous_swap = Some((input, output, now));
            }
            Err(error) => {
                self.previous_swap = None;
                failed(data, "swap_io", "/proc/vmstat", &error);
            }
        }
    }

    fn disks(&mut self, now: Instant, data: &mut MonitoringData) {
        let counters = fs::read_to_string("/proc/diskstats").and_then(|text| parse_disks(&text));
        match counters {
            Ok(counters) => {
                let mut next = HashMap::new();
                for current in counters.into_iter().take(128) {
                    let previous = self.previous_disks.get(&current.id);
                    data.disk_io.push(disk_metric(&current, previous, now));
                    next.insert(current.id.clone(), (current, now));
                }
                self.previous_disks = next;
                let status = if data.disk_io.iter().any(|v| v.status == MetricStatus::Ok) {
                    MetricStatus::Ok
                } else if data.disk_io.is_empty() {
                    MetricStatus::Unavailable
                } else {
                    MetricStatus::WarmingUp
                };
                data.capabilities
                    .insert("disk_io".into(), capability(status, "/proc/diskstats"));
            }
            Err(error) => {
                self.previous_disks.clear();
                failed(data, "disk_io", "/proc/diskstats", &error);
            }
        }
    }

    fn network(&mut self, now: Instant, data: &mut MonitoringData) {
        let counters = fs::read_to_string("/proc/net/dev").and_then(|text| parse_network(&text));
        match counters {
            Ok(counters) => {
                let mut next = HashMap::new();
                for current in counters.into_iter().take(128) {
                    let name = &current.name;
                    let ifindex = fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
                        .ok()
                        .and_then(|text| text.trim().parse::<u32>().ok());
                    let id = ifindex
                        .map(|index| format!("ifindex:{index}"))
                        .unwrap_or_else(|| format!("name:{name}"));
                    let previous = self.previous_network.get(&id);
                    let mut value = network_metric(&current, previous, now);
                    value.id = id.clone();
                    value.link_up = fs::read_to_string(format!("/sys/class/net/{name}/operstate"))
                        .ok()
                        .and_then(|state| match state.trim() {
                            "up" => Some(true),
                            "down" | "lowerlayerdown" => Some(false),
                            _ => None,
                        });
                    data.network_health.push(value);
                    next.insert(id, (current, now));
                }
                self.previous_network = next;
                data.capabilities.insert(
                    "network_health".into(),
                    capability(
                        if data.network_health.is_empty() {
                            MetricStatus::Unavailable
                        } else {
                            MetricStatus::Ok
                        },
                        "/proc/net/dev",
                    ),
                );
            }
            Err(error) => {
                self.previous_network.clear();
                failed(data, "network_health", "/proc/net/dev", &error);
            }
        }
    }

    fn tcp(&mut self, data: &mut MonitoringData) {
        let result = fs::read_to_string("/proc/net/tcp").and_then(|v4| {
            let v6 = match fs::read_to_string("/proc/net/tcp6") {
                Ok(value) => value,
                Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
                Err(error) => return Err(error),
            };
            parse_tcp(&v4, &v6)
        });
        match result {
            Ok(tcp) => {
                data.tcp = tcp;
                data.capabilities.insert(
                    "tcp".into(),
                    capability(MetricStatus::Ok, "/proc/net/tcp,tcp6"),
                );
            }
            Err(error) => failed(data, "tcp", "/proc/net/tcp,tcp6", &error),
        }
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn failed(data: &mut MonitoringData, key: &str, source: &str, error: &io::Error) {
    let mut value = capability(
        if error.kind() == io::ErrorKind::PermissionDenied {
            MetricStatus::PermissionDenied
        } else {
            MetricStatus::Unavailable
        },
        source,
    );
    value.error = Some(error.to_string());
    data.capabilities.insert(key.into(), value);
}

fn parse_cpu(text: &str) -> io::Result<[u64; 8]> {
    let line = text
        .lines()
        .find(|line| line.starts_with("cpu "))
        .ok_or_else(|| invalid("missing CPU counters"))?;
    let mut counters = [0; 8];
    let fields = line.split_whitespace().skip(1).collect::<Vec<_>>();
    if fields.len() < 4 {
        return Err(invalid("incomplete CPU counters"));
    }
    for (index, field) in fields.iter().take(8).enumerate() {
        counters[index] = field.parse().map_err(|_| invalid("invalid CPU counter"))?;
    }
    Ok(counters)
}

fn cpu_delta(previous: [u64; 8], current: [u64; 8]) -> Option<CpuTimes> {
    let mut delta = [0u64; 8];
    for index in 0..8 {
        delta[index] = current[index].checked_sub(previous[index])?;
    }
    let total = delta
        .iter()
        .try_fold(0u64, |sum, value| sum.checked_add(*value))?;
    if total == 0 {
        return None;
    }
    let percent = |value: u64| value as f64 * 100.0 / total as f64;
    Some(CpuTimes {
        user_percent: Some(percent(delta[0].checked_add(delta[1])?)),
        system_percent: Some(percent(
            delta[2].checked_add(delta[5])?.checked_add(delta[6])?,
        )),
        iowait_percent: Some(percent(delta[4])),
        steal_percent: Some(percent(delta[7])),
    })
}

fn parse_meminfo(text: &str) -> io::Result<BTreeMap<String, u64>> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        if !matches!(key, "MemAvailable" | "Cached" | "Buffers") {
            continue;
        }
        let mut fields = value.split_whitespace();
        let count = fields
            .next()
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| invalid("invalid memory counter"))?;
        let count = match fields.next() {
            Some("kB") => count.checked_mul(1024),
            None => Some(count),
            _ => None,
        }
        .ok_or_else(|| invalid("invalid memory counter unit"))?;
        values.insert(key.into(), count);
    }
    Ok(values)
}

fn parse_swap(text: &str) -> io::Result<(u64, u64)> {
    let mut input = None;
    let mut output = None;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("pswpin") => input = fields.next().and_then(|v| v.parse().ok()),
            Some("pswpout") => output = fields.next().and_then(|v| v.parse().ok()),
            _ => {}
        }
    }
    input
        .zip(output)
        .ok_or_else(|| invalid("missing swap counters"))
}

fn parse_disks(text: &str) -> io::Result<Vec<DiskCounters>> {
    let mut result = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 14 {
            return Err(invalid("incomplete disk counters"));
        }
        let major = fields[0]
            .parse::<u32>()
            .map_err(|_| invalid("invalid disk major"))?;
        let minor = fields[1]
            .parse::<u32>()
            .map_err(|_| invalid("invalid disk minor"))?;
        let name = fields[2];
        // Linux device names cannot contain path separators. Exclude synthetic RAM/loop devices.
        if name.contains('/') || name.starts_with("loop") || name.starts_with("ram") {
            continue;
        }
        let mut values = [0; 11];
        for (index, value) in fields[3..14].iter().enumerate() {
            values[index] = value.parse().map_err(|_| invalid("invalid disk counter"))?;
        }
        result.push(DiskCounters {
            id: format!("{major}:{minor}"),
            name: name.into(),
            values,
        });
    }
    Ok(result)
}

fn disk_metric(
    current: &DiskCounters,
    previous: Option<&(DiskCounters, Instant)>,
    now: Instant,
) -> DiskIo {
    let c = &current.values;
    let mut metric = DiskIo {
        id: current.id.clone(),
        name: current.name.clone(),
        status: MetricStatus::WarmingUp,
        reads: Some(c[0].to_string()),
        writes: Some(c[4].to_string()),
        read_bytes: c[2].checked_mul(512).map(|v| v.to_string()),
        written_bytes: c[6].checked_mul(512).map(|v| v.to_string()),
        ..Default::default()
    };
    let Some((previous, at)) = previous else {
        return metric;
    };
    let p = &previous.values;
    let elapsed = now.duration_since(*at).as_secs_f64();
    if elapsed <= 0.0 || [0, 2, 3, 4, 6, 7, 9, 10].iter().any(|&i| c[i] < p[i]) {
        return metric;
    }
    metric.status = MetricStatus::Ok;
    metric.read_iops = rate(p[0], c[0], elapsed);
    metric.write_iops = rate(p[4], c[4], elapsed);
    metric.read_bytes_per_sec = rate(p[2], c[2], elapsed).map(|value| value * 512.0);
    metric.write_bytes_per_sec = rate(p[6], c[6], elapsed).map(|value| value * 512.0);
    metric.read_latency_ms = (c[0] > p[0]).then(|| (c[3] - p[3]) as f64 / (c[0] - p[0]) as f64);
    metric.write_latency_ms = (c[4] > p[4]).then(|| (c[7] - p[7]) as f64 / (c[4] - p[4]) as f64);
    metric.utilization_percent = Some(((c[9] - p[9]) as f64 / (elapsed * 10.0)).clamp(0.0, 100.0));
    metric.queue_depth = Some((c[10] - p[10]) as f64 / (elapsed * 1000.0));
    metric
}

fn parse_network(text: &str) -> io::Result<Vec<NetworkCounters>> {
    let mut result = Vec::new();
    for line in text.lines() {
        let Some((name, counters)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.contains('/') || name.contains("..") {
            return Err(invalid("invalid network interface name"));
        }
        let fields = counters.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 16 {
            return Err(invalid("incomplete network counters"));
        }
        let mut values = [0; 16];
        for (index, field) in fields.iter().enumerate() {
            values[index] = field
                .parse()
                .map_err(|_| invalid("invalid network counter"))?;
        }
        result.push(NetworkCounters {
            name: name.into(),
            values,
        });
    }
    Ok(result)
}

fn network_metric(
    current: &NetworkCounters,
    previous: Option<&(NetworkCounters, Instant)>,
    now: Instant,
) -> NetworkHealth {
    let c = &current.values;
    let mut value = NetworkHealth {
        id: current.name.clone(),
        name: current.name.clone(),
        status: MetricStatus::WarmingUp,
        received_bytes: c[0].to_string(),
        received_packets: c[1].to_string(),
        receive_errors: c[2].to_string(),
        receive_drops: Some(c[3].to_string()),
        transmitted_bytes: c[8].to_string(),
        transmitted_packets: c[9].to_string(),
        transmit_errors: c[10].to_string(),
        transmit_drops: Some(c[11].to_string()),
        ..Default::default()
    };
    if let Some((previous, at)) = previous {
        let p = &previous.values;
        let elapsed = now.duration_since(*at).as_secs_f64();
        if elapsed > 0.0 && [0, 1, 2, 3, 8, 9, 10, 11].iter().all(|&i| c[i] >= p[i]) {
            value.status = MetricStatus::Ok;
            value.receive_errors_per_sec = rate(p[2], c[2], elapsed);
            value.transmit_errors_per_sec = rate(p[10], c[10], elapsed);
            value.receive_drops_per_sec = rate(p[3], c[3], elapsed);
            value.transmit_drops_per_sec = rate(p[11], c[11], elapsed);
        }
    }
    value
}

fn inodes(disk: &DiskMetric) -> InodeMetric {
    let mut result = InodeMetric {
        id: disk.mount_point.clone(),
        mount_point: disk.mount_point.clone(),
        ..Default::default()
    };
    let Ok(path) = CString::new(disk.mount_point.as_bytes()) else {
        result.status = MetricStatus::Unavailable;
        return result;
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        result.status = if io::Error::last_os_error().kind() == io::ErrorKind::PermissionDenied {
            MetricStatus::PermissionDenied
        } else {
            MetricStatus::Unavailable
        };
        return result;
    }
    let stats = unsafe { stats.assume_init() };
    let total = stats.f_files as u64;
    let free = stats.f_ffree as u64;
    if total == 0 || free > total {
        return result;
    }
    result.status = MetricStatus::Ok;
    result.total = Some(total.to_string());
    result.free = Some(free.to_string());
    result.used = Some((total - free).to_string());
    result.used_percent = Some((total - free) as f64 * 100.0 / total as f64);
    result
}

fn parse_tcp(v4: &str, v6: &str) -> io::Result<TcpMetrics> {
    let mut value = TcpMetrics {
        scope: "network_namespace".into(),
        ..Default::default()
    };
    let mut ports = BTreeSet::new();
    for text in [v4, v6] {
        for line in text.lines().skip(1).filter(|line| !line.trim().is_empty()) {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 4 {
                return Err(invalid("incomplete TCP table"));
            }
            let state =
                u8::from_str_radix(fields[3], 16).map_err(|_| invalid("invalid TCP state"))?;
            let name = match state {
                1 => "established",
                2 => "syn_sent",
                3 => "syn_received",
                4 => "fin_wait_1",
                5 => "fin_wait_2",
                6 => "time_wait",
                7 => "closed",
                8 => "close_wait",
                9 => "last_ack",
                10 => "listen",
                11 => "closing",
                12 => "new_syn_received",
                _ => "unknown",
            };
            *value.states.entry(name.into()).or_insert(0) += 1;
            if state == 10 {
                let (_, port) = fields[1]
                    .rsplit_once(':')
                    .ok_or_else(|| invalid("invalid TCP endpoint"))?;
                ports.insert(
                    u16::from_str_radix(port, 16).map_err(|_| invalid("invalid TCP port"))?,
                );
                value.listening_sockets += 1;
            }
        }
    }
    value.listening_ports = ports.len() as u64;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_ignores_guest_duplicate_and_handles_reset() {
        let counters = parse_cpu("cpu 10 2 3 20 4 5 6 7 100 100\n").unwrap();
        let times = cpu_delta([0; 8], counters).unwrap();
        assert!((times.user_percent.unwrap() - 12.0 / 57.0 * 100.0).abs() < 0.001);
        assert!(cpu_delta(counters, [0; 8]).is_none());
        assert!(cpu_delta(counters, counters).is_none());
    }
    #[test]
    fn swap_and_memory_have_correct_units() {
        assert_eq!(parse_swap("pswpin 3\npswpout 7\n").unwrap(), (3, 7));
        assert!(parse_swap("pswpin 3\n").is_err());
        assert_eq!(
            parse_meminfo("MemAvailable: 42 kB\nCached: 3 kB\n").unwrap()["MemAvailable"],
            42 * 1024
        );
    }
    #[test]
    fn disk_io_uses_sectors_and_completion_latency() {
        let p = parse_disks("8 0 sda 1 0 2 5 3 0 4 6 0 10 20")
            .unwrap()
            .remove(0);
        let c = parse_disks("8 0 sda 5 0 10 13 7 0 12 14 0 20 60")
            .unwrap()
            .remove(0);
        let at = Instant::now();
        assert_eq!(disk_metric(&c, None, at).read_iops, None);
        let value = disk_metric(
            &c,
            Some(&(p.clone(), at)),
            at + std::time::Duration::from_secs(2),
        );
        assert_eq!(value.read_bytes_per_sec, Some(2048.0));
        assert_eq!(value.read_iops, Some(2.0));
        assert_eq!(value.read_latency_ms, Some(2.0));
        assert_eq!(
            disk_metric(&p, Some(&(c, at)), at + std::time::Duration::from_secs(2)).status,
            MetricStatus::WarmingUp
        );
    }
    #[test]
    fn network_errors_are_distinct_from_drops() {
        let values = parse_network("eth0: 100 2 3 4 0 0 0 0 200 5 6 7 0 0 0 0").unwrap();
        let value = network_metric(&values[0], None, Instant::now());
        assert_eq!(value.receive_errors, "3");
        assert_eq!(value.receive_drops.as_deref(), Some("4"));
        assert!(value.receive_errors_per_sec.is_none());
    }
    #[test]
    fn tcp_deduplicates_ports_across_ipv4_ipv6() {
        let tcp = parse_tcp("header\n0: 0100007F:0050 00000000:0000 0A\n1: 0100007F:1234 00000000:0000 06", "header\n0: 00000000000000000000000000000000:0050 00000000000000000000000000000000:0000 0A").unwrap();
        assert_eq!(tcp.listening_sockets, 2);
        assert_eq!(tcp.listening_ports, 1);
        assert_eq!(tcp.states["time_wait"], 1);
    }
    #[test]
    fn malformed_proc_counters_never_become_zero_measurements() {
        assert!(parse_cpu("cpu 1 2 malformed 4").is_err());
        assert!(parse_disks("8 0 sda 1 2 3").is_err());
        assert!(parse_network("eth0: 1 2 3").is_err());
        assert!(parse_network("../bad: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0").is_err());
        assert!(parse_tcp("header\n0: endpoint remote unknown", "").is_err());
        assert!(parse_meminfo("Cached: 18446744073709551615 kB").is_err());
    }
    #[test]
    fn idle_disk_has_zero_rates_and_no_completion_latency() {
        let disk = parse_disks("259 0 nvme0n1 1 0 2 5 3 0 4 6 0 10 20 0 0 0 0")
            .unwrap()
            .remove(0);
        let at = Instant::now();
        let value = disk_metric(
            &disk,
            Some(&(disk.clone(), at)),
            at + std::time::Duration::from_secs(1),
        );
        assert_eq!(value.status, MetricStatus::Ok);
        assert_eq!(value.read_iops, Some(0.0));
        assert_eq!(value.read_latency_ms, None);
        assert_eq!(value.queue_depth, Some(0.0));
    }
    #[test]
    fn interface_counter_reset_invalidates_all_rates() {
        let previous = parse_network("eth0: 100 2 3 4 0 0 0 0 200 5 6 7 0 0 0 0")
            .unwrap()
            .remove(0);
        let current = parse_network("eth0: 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0")
            .unwrap()
            .remove(0);
        let at = Instant::now();
        let value = network_metric(
            &current,
            Some(&(previous, at)),
            at + std::time::Duration::from_secs(1),
        );
        assert_eq!(value.status, MetricStatus::WarmingUp);
        assert!(value.receive_drops_per_sec.is_none());
    }
    #[test]
    fn native_kernel_reports_counters_and_mount_inode_without_root() {
        let directory = tempfile::tempdir().unwrap();
        let disk = DiskMetric {
            name: "test".into(),
            mount_point: directory.path().to_string_lossy().into_owned(),
            file_system: "test".into(),
            total_bytes: 0,
            used_bytes: 0,
        };
        let mut collector = PlatformCollector::new();
        let mut first = MonitoringData::default();
        collector.collect(Instant::now(), &[disk.clone()], &mut first);
        assert_eq!(
            first.capabilities["cpu_times"].status,
            MetricStatus::WarmingUp
        );
        assert_eq!(
            first.capabilities["swap_io"].status,
            MetricStatus::WarmingUp
        );
        assert_eq!(first.capabilities["tcp"].status, MetricStatus::Ok);
        assert_eq!(first.tcp.scope, "network_namespace");
        assert!(!first.network_health.is_empty());
        assert!(
            first
                .network_health
                .iter()
                .all(|v| v.receive_errors_per_sec.is_none())
        );
        assert_eq!(first.inodes[0].status, MetricStatus::Ok);
        std::thread::sleep(std::time::Duration::from_millis(150));
        let mut second = MonitoringData::default();
        collector.collect(Instant::now(), &[disk], &mut second);
        assert_eq!(second.capabilities["cpu_times"].status, MetricStatus::Ok);
        assert_eq!(second.capabilities["swap_io"].status, MetricStatus::Ok);
        assert!(
            second
                .network_health
                .iter()
                .all(|v| v.receive_errors_per_sec.is_some())
        );
        assert!(second.inodes[0].used_percent.unwrap().is_finite());
    }
}
