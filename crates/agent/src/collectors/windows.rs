use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::mem::{offset_of, size_of};
use std::ptr;
use std::time::Instant;

use pinglake_protocol::{
    DiskIo, DiskMetric, InodeMetric, MetricStatus, MonitoringData, NetworkHealth, TcpMetrics,
};
use windows_sys::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER};
use windows_sys::Win32::NetworkManagement::IpHelper::*;
use windows_sys::Win32::System::Performance::*;

use super::{capability, rate};

pub struct PlatformCollector {
    previous_network: HashMap<String, ([u64; 8], Instant)>,
    previous_disks: HashMap<String, [u64; 4]>,
    pdh: Option<PdhCollector>,
}

impl PlatformCollector {
    pub fn new() -> Self {
        Self {
            previous_network: HashMap::new(),
            previous_disks: HashMap::new(),
            pdh: None,
        }
    }

    pub fn collect(&mut self, now: Instant, disks: &[DiskMetric], data: &mut MonitoringData) {
        for name in [
            "swap_io",
            "inodes",
            "cpu_iowait",
            "cpu_steal",
            "memory_cached",
            "memory_buffers",
        ] {
            data.capabilities.insert(
                name.into(),
                capability(MetricStatus::Unsupported, "windows"),
            );
        }
        data.inodes = disks
            .iter()
            .map(|disk| InodeMetric {
                id: disk.mount_point.clone(),
                mount_point: disk.mount_point.clone(),
                status: MetricStatus::Unsupported,
                ..Default::default()
            })
            .collect();
        self.network(now, data);
        match tcp_metrics() {
            Ok(value) => {
                data.tcp = value;
                data.capabilities
                    .insert("tcp".into(), capability(MetricStatus::Ok, "IPHelper"));
            }
            Err(code) => failed(data, "tcp", "IPHelper", code),
        }
        self.performance(data);
    }

    fn network(&mut self, now: Instant, data: &mut MonitoringData) {
        let mut table = ptr::null_mut();
        let result = unsafe { GetIfTable2(&mut table) };
        if result != 0 {
            self.previous_network.clear();
            failed(data, "network_health", "GetIfTable2", result);
            return;
        }
        if table.is_null() {
            failed(data, "network_health", "GetIfTable2", 13);
            return;
        }
        let allocation = MibAllocation(table.cast());
        let rows = unsafe {
            std::slice::from_raw_parts(
                ptr::addr_of!((*table).Table).cast::<MIB_IF_ROW2>(),
                (*table).NumEntries as usize,
            )
        };
        let mut next = HashMap::new();
        for row in rows.iter().take(128) {
            let guid = &row.InterfaceGuid;
            let id = format!(
                "{:08x}-{:04x}-{:04x}-{}",
                guid.data1,
                guid.data2,
                guid.data3,
                guid.data4
                    .iter()
                    .map(|v| format!("{v:02x}"))
                    .collect::<String>()
            );
            let c = [
                row.InOctets,
                row.OutOctets,
                row.InUcastPkts.saturating_add(row.InNUcastPkts),
                row.OutUcastPkts.saturating_add(row.OutNUcastPkts),
                row.InErrors,
                row.OutErrors,
                row.InDiscards,
                row.OutDiscards,
            ];
            let name = String::from_utf16_lossy(
                &row.Alias[..row
                    .Alias
                    .iter()
                    .position(|&v| v == 0)
                    .unwrap_or(row.Alias.len())],
            );
            let mut value = NetworkHealth {
                id: id.clone(),
                name,
                status: MetricStatus::WarmingUp,
                link_up: Some(row.OperStatus == 1),
                received_bytes: c[0].to_string(),
                transmitted_bytes: c[1].to_string(),
                received_packets: c[2].to_string(),
                transmitted_packets: c[3].to_string(),
                receive_errors: c[4].to_string(),
                transmit_errors: c[5].to_string(),
                receive_drops: Some(c[6].to_string()),
                transmit_drops: Some(c[7].to_string()),
                ..Default::default()
            };
            if let Some((p, at)) = self.previous_network.get(&id) {
                let elapsed = now.duration_since(*at).as_secs_f64();
                if elapsed > 0.0 && (0..8).all(|index| c[index] >= p[index]) {
                    value.status = MetricStatus::Ok;
                    value.receive_errors_per_sec = rate(p[4], c[4], elapsed);
                    value.transmit_errors_per_sec = rate(p[5], c[5], elapsed);
                    value.receive_drops_per_sec = rate(p[6], c[6], elapsed);
                    value.transmit_drops_per_sec = rate(p[7], c[7], elapsed);
                }
            }
            data.network_health.push(value);
            next.insert(id, (c, now));
        }
        drop(allocation);
        self.previous_network = next;
        data.capabilities.insert(
            "network_health".into(),
            capability(MetricStatus::Ok, "GetIfTable2"),
        );
    }

    fn performance(&mut self, data: &mut MonitoringData) {
        if self.pdh.is_none() {
            match PdhCollector::new() {
                Ok(query) => self.pdh = Some(query),
                Err(code) => {
                    failed(data, "cpu_times", "PDH", code);
                    failed(data, "disk_io", "PDH", code);
                    return;
                }
            }
        }
        let pdh = self.pdh.as_mut().expect("PDH initialized");
        let code = unsafe { PdhCollectQueryData(pdh.query as _) };
        if code != 0 {
            failed(data, "cpu_times", "PDH", code);
            failed(data, "disk_io", "PDH", code);
            self.pdh = None;
            self.previous_disks.clear();
            return;
        }
        pdh.samples += 1;
        data.cpu_times.user_percent = pdh.value("cpu_user").map(|v| v.clamp(0.0, 100.0));
        data.cpu_times.system_percent = pdh.value("cpu_system").map(|v| v.clamp(0.0, 100.0));
        let cpu_status =
            if data.cpu_times.user_percent.is_some() && data.cpu_times.system_percent.is_some() {
                MetricStatus::Ok
            } else if pdh.samples == 1
                && pdh.counters.contains_key("cpu_user")
                && pdh.counters.contains_key("cpu_system")
            {
                MetricStatus::WarmingUp
            } else {
                MetricStatus::Unavailable
            };
        data.capabilities.insert(
            "cpu_times".into(),
            capability(cpu_status, "PDH English Processor Information"),
        );
        let raw = ["read_bytes", "write_bytes", "reads", "writes"].map(|key| pdh.raw_array(key));
        let formatted = [
            "read_bytes",
            "write_bytes",
            "reads",
            "writes",
            "read_latency",
            "write_latency",
            "idle",
            "queue",
        ]
        .map(|key| pdh.array(key));
        let mut next = HashMap::new();
        let Some(raw_bytes) = raw[0].as_ref() else {
            if let Some(code) = pdh.errors.get("read_bytes") {
                failed(data, "disk_io", "PDH PhysicalDisk", *code);
            } else {
                data.capabilities.insert(
                    "disk_io".into(),
                    capability(MetricStatus::Unavailable, "PDH PhysicalDisk"),
                );
            }
            self.previous_disks.clear();
            return;
        };
        for name in raw_bytes
            .keys()
            .filter(|name| name.as_str() != "_Total")
            .take(128)
        {
            let counts = raw
                .each_ref()
                .map(|values| values.as_ref().and_then(|v| v.get(name)).copied());
            let mut metric = DiskIo {
                id: format!(
                    "physicaldisk:{}",
                    name.split_whitespace().next().unwrap_or(name)
                ),
                name: name.clone(),
                status: MetricStatus::WarmingUp,
                read_bytes: counts[0].map(|v| v.to_string()),
                written_bytes: counts[1].map(|v| v.to_string()),
                reads: counts[2].map(|v| v.to_string()),
                writes: counts[3].map(|v| v.to_string()),
                ..Default::default()
            };
            if let [
                Some(read_bytes),
                Some(write_bytes),
                Some(reads),
                Some(writes),
            ] = counts
            {
                let current = [read_bytes, write_bytes, reads, writes];
                if self
                    .previous_disks
                    .get(name)
                    .is_some_and(|p| (0..4).all(|i| current[i] >= p[i]))
                {
                    let values = formatted
                        .each_ref()
                        .map(|array| array.as_ref().and_then(|values| values.get(name)).copied());
                    metric.read_bytes_per_sec = values[0];
                    metric.write_bytes_per_sec = values[1];
                    metric.read_iops = values[2];
                    metric.write_iops = values[3];
                    metric.read_latency_ms = values[4]
                        .filter(|_| metric.read_iops.is_some_and(|v| v > 0.0))
                        .map(|v| v * 1000.0);
                    metric.write_latency_ms = values[5]
                        .filter(|_| metric.write_iops.is_some_and(|v| v > 0.0))
                        .map(|v| v * 1000.0);
                    metric.utilization_percent = values[6].map(|v| (100.0 - v).clamp(0.0, 100.0));
                    metric.queue_depth = values[7];
                    metric.status = if values[..4].iter().all(Option::is_some) {
                        MetricStatus::Ok
                    } else {
                        MetricStatus::Unavailable
                    };
                }
                next.insert(name.clone(), current);
            } else {
                metric.status = MetricStatus::Unavailable;
            }
            data.disk_io.push(metric);
        }
        self.previous_disks = next;
        let status = if data.disk_io.iter().any(|v| v.status == MetricStatus::Ok) {
            MetricStatus::Ok
        } else if !data.disk_io.is_empty()
            && data
                .disk_io
                .iter()
                .all(|v| v.status == MetricStatus::WarmingUp)
        {
            MetricStatus::WarmingUp
        } else {
            MetricStatus::Unavailable
        };
        data.capabilities.insert(
            "disk_io".into(),
            capability(status, "PDH English PhysicalDisk"),
        );
    }
}

struct MibAllocation(*mut std::ffi::c_void);
impl Drop for MibAllocation {
    fn drop(&mut self) {
        unsafe { FreeMibTable(self.0) };
    }
}

fn failed(data: &mut MonitoringData, key: &str, source: &str, code: u32) {
    let mut value = capability(
        if code == ERROR_ACCESS_DENIED || code == PDH_ACCESS_DENIED {
            MetricStatus::PermissionDenied
        } else {
            MetricStatus::Unavailable
        },
        source,
    );
    value.error = Some(format!("native error 0x{code:08x}"));
    data.capabilities.insert(key.into(), value);
}

fn table_buffer(
    mut fetch: impl FnMut(*mut std::ffi::c_void, &mut u32) -> u32,
) -> Result<(Vec<u64>, usize), u32> {
    let mut bytes = 0;
    let result = fetch(ptr::null_mut(), &mut bytes);
    if result != 0 && result != ERROR_INSUFFICIENT_BUFFER {
        return Err(result);
    }
    for _ in 0..3 {
        if !(4..=16 * 1024 * 1024).contains(&bytes) {
            return Err(13);
        }
        let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
        let result = fetch(buffer.as_mut_ptr().cast(), &mut bytes);
        if result == 0 {
            return Ok((buffer, bytes as usize));
        }
        if result != ERROR_INSUFFICIENT_BUFFER {
            return Err(result);
        }
    }
    Err(ERROR_INSUFFICIENT_BUFFER)
}

fn tcp_metrics() -> Result<TcpMetrics, u32> {
    let mut result = TcpMetrics {
        scope: "host".into(),
        ..Default::default()
    };
    let mut ports = BTreeSet::new();
    let (v4, bytes) = table_buffer(|table, size| unsafe { GetTcpTable(table.cast(), size, 0) })?;
    let pointer = v4.as_ptr().cast::<MIB_TCPTABLE>();
    let count = unsafe { (*pointer).dwNumEntries as usize };
    if count > bytes.saturating_sub(offset_of!(MIB_TCPTABLE, table)) / size_of::<MIB_TCPROW_LH>() {
        return Err(13);
    }
    let rows = unsafe {
        std::slice::from_raw_parts(
            ptr::addr_of!((*pointer).table).cast::<MIB_TCPROW_LH>(),
            count,
        )
    };
    for row in rows {
        record_tcp(
            &mut result,
            &mut ports,
            unsafe { row.Anonymous.dwState },
            row.dwLocalPort,
        );
    }
    let (v6, bytes) = table_buffer(|table, size| unsafe { GetTcp6Table(table.cast(), size, 0) })?;
    let pointer = v6.as_ptr().cast::<MIB_TCP6TABLE>();
    let count = unsafe { (*pointer).dwNumEntries as usize };
    if count > bytes.saturating_sub(offset_of!(MIB_TCP6TABLE, table)) / size_of::<MIB_TCP6ROW>() {
        return Err(13);
    }
    let rows = unsafe {
        std::slice::from_raw_parts(ptr::addr_of!((*pointer).table).cast::<MIB_TCP6ROW>(), count)
    };
    for row in rows {
        record_tcp(&mut result, &mut ports, row.State as u32, row.dwLocalPort);
    }
    result.listening_ports = ports.len() as u64;
    result.listening_port_numbers = ports.into_iter().collect();
    Ok(result)
}

fn record_tcp(result: &mut TcpMetrics, ports: &mut BTreeSet<u16>, state: u32, port: u32) {
    let state_name = match state {
        1 => "closed",
        2 => "listen",
        3 => "syn_sent",
        4 => "syn_received",
        5 => "established",
        6 => "fin_wait_1",
        7 => "fin_wait_2",
        8 => "close_wait",
        9 => "closing",
        10 => "last_ack",
        11 => "time_wait",
        12 => "delete_tcb",
        _ => "unknown",
    };
    *result.states.entry(state_name.into()).or_insert(0) += 1;
    if state == 2 {
        result.listening_sockets += 1;
        ports.insert(u16::from_be(port as u16));
    }
}

struct PdhCollector {
    query: usize,
    counters: HashMap<&'static str, usize>,
    errors: HashMap<&'static str, u32>,
    samples: u64,
}

impl PdhCollector {
    fn new() -> Result<Self, u32> {
        let mut query = ptr::null_mut();
        let status = unsafe { PdhOpenQueryW(ptr::null(), 0, &mut query) };
        if status != 0 {
            return Err(status);
        }
        let mut result = Self {
            query: query as usize,
            counters: HashMap::new(),
            errors: HashMap::new(),
            samples: 0,
        };
        for (key, path) in [
            ("cpu_user", r"\Processor Information(_Total)\% User Time"),
            (
                "cpu_system",
                r"\Processor Information(_Total)\% Privileged Time",
            ),
            ("read_bytes", r"\PhysicalDisk(*)\Disk Read Bytes/sec"),
            ("write_bytes", r"\PhysicalDisk(*)\Disk Write Bytes/sec"),
            ("reads", r"\PhysicalDisk(*)\Disk Reads/sec"),
            ("writes", r"\PhysicalDisk(*)\Disk Writes/sec"),
            ("read_latency", r"\PhysicalDisk(*)\Avg. Disk sec/Read"),
            ("write_latency", r"\PhysicalDisk(*)\Avg. Disk sec/Write"),
            ("idle", r"\PhysicalDisk(*)\% Idle Time"),
            ("queue", r"\PhysicalDisk(*)\Current Disk Queue Length"),
        ] {
            let name = path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let mut handle = ptr::null_mut();
            let code = unsafe { PdhAddEnglishCounterW(query, name.as_ptr(), 0, &mut handle) };
            if code == 0 {
                result.counters.insert(key, handle as usize);
            } else {
                result.errors.insert(key, code);
            }
        }
        Ok(result)
    }

    fn value(&self, key: &str) -> Option<f64> {
        let &handle = self.counters.get(key)?;
        let mut value = PDH_FMT_COUNTERVALUE::default();
        let code = unsafe {
            PdhGetFormattedCounterValue(handle as _, PDH_FMT_DOUBLE, ptr::null_mut(), &mut value)
        };
        let number = unsafe { value.Anonymous.doubleValue };
        (code == 0 && valid_status(value.CStatus) && number.is_finite() && number >= 0.0)
            .then_some(number)
    }

    fn array(&self, key: &str) -> Option<BTreeMap<String, f64>> {
        let &handle = self.counters.get(key)?;
        let (buffer, bytes, count) = pdh_buffer(|pointer, bytes, count| unsafe {
            PdhGetFormattedCounterArrayW(handle as _, PDH_FMT_DOUBLE, bytes, count, pointer.cast())
        })?;
        if count > bytes / size_of::<PDH_FMT_COUNTERVALUE_ITEM_W>() {
            return None;
        }
        let rows = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>(), count)
        };
        let mut values = BTreeMap::new();
        for row in rows {
            let number = unsafe { row.FmtValue.Anonymous.doubleValue };
            if valid_status(row.FmtValue.CStatus)
                && number.is_finite()
                && number >= 0.0
                && let Some(name) = bounded_name(row.szName, &buffer)
            {
                values.insert(name, number);
            }
        }
        Some(values)
    }

    fn raw_array(&self, key: &str) -> Option<BTreeMap<String, u64>> {
        let &handle = self.counters.get(key)?;
        let (buffer, bytes, count) = pdh_buffer(|pointer, bytes, count| unsafe {
            PdhGetRawCounterArrayW(handle as _, bytes, count, pointer.cast())
        })?;
        if count > bytes / size_of::<PDH_RAW_COUNTER_ITEM_W>() {
            return None;
        }
        let rows = unsafe {
            std::slice::from_raw_parts(buffer.as_ptr().cast::<PDH_RAW_COUNTER_ITEM_W>(), count)
        };
        let mut values = BTreeMap::new();
        for row in rows {
            if valid_status(row.RawValue.CStatus)
                && let (Some(name), Ok(number)) = (
                    bounded_name(row.szName, &buffer),
                    u64::try_from(row.RawValue.FirstValue),
                )
            {
                values.insert(name, number);
            }
        }
        Some(values)
    }
}

impl Drop for PdhCollector {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query as _) };
    }
}
fn valid_status(code: u32) -> bool {
    code == PDH_CSTATUS_VALID_DATA || code == PDH_CSTATUS_NEW_DATA
}

fn pdh_buffer(
    mut fetch: impl FnMut(*mut std::ffi::c_void, &mut u32, &mut u32) -> u32,
) -> Option<(Vec<u64>, usize, usize)> {
    let mut bytes = 0;
    let mut count = 0;
    if fetch(ptr::null_mut(), &mut bytes, &mut count) != PDH_MORE_DATA {
        return None;
    }
    for _ in 0..3 {
        if bytes == 0 || bytes > 4 * 1024 * 1024 {
            return None;
        }
        let mut buffer = vec![0u64; (bytes as usize).div_ceil(8)];
        let code = fetch(buffer.as_mut_ptr().cast(), &mut bytes, &mut count);
        if code == 0 {
            return Some((buffer, bytes as usize, count as usize));
        }
        if code != PDH_MORE_DATA {
            return None;
        }
    }
    None
}

fn bounded_name(pointer: *const u16, buffer: &[u64]) -> Option<String> {
    let start = buffer.as_ptr() as usize;
    let end = start + std::mem::size_of_val(buffer);
    let address = pointer as usize;
    if address < start || address >= end || !address.is_multiple_of(2) {
        return None;
    }
    let maximum = ((end - address) / 2).min(512);
    let values = unsafe { std::slice::from_raw_parts(pointer, maximum) };
    let length = values.iter().position(|&value| value == 0)?;
    Some(String::from_utf16_lossy(&values[..length]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tcp_ports_are_network_order_and_distinct_across_families() {
        let mut metrics = TcpMetrics::default();
        let mut ports = BTreeSet::new();
        record_tcp(&mut metrics, &mut ports, 2, 80u16.to_be() as u32);
        record_tcp(&mut metrics, &mut ports, 2, 80u16.to_be() as u32);
        record_tcp(&mut metrics, &mut ports, 11, 100u16.to_be() as u32);
        assert_eq!(metrics.listening_sockets, 2);
        assert_eq!(ports, BTreeSet::from([80]));
        assert_eq!(metrics.states["time_wait"], 1);
    }
    #[test]
    fn native_tcp_observes_live_loopback_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let metrics = tcp_metrics().unwrap();
        assert!(metrics.listening_sockets > 0);
        assert!(metrics.states.get("listen").is_some_and(|&count| count > 0));
        drop(listener);
    }
    #[test]
    fn native_counters_provide_network_and_explicit_platform_status() {
        let mut collector = PlatformCollector::new();
        let mut data = MonitoringData::default();
        collector.collect(Instant::now(), &[], &mut data);
        assert_eq!(
            data.capabilities["inodes"].status,
            MetricStatus::Unsupported
        );
        assert_eq!(
            data.capabilities["swap_io"].status,
            MetricStatus::Unsupported
        );
        assert_eq!(data.capabilities["network_health"].status, MetricStatus::Ok);
        assert!(!data.network_health.is_empty());
        assert!(
            data.network_health
                .iter()
                .all(|v| v.receive_errors_per_sec.is_none())
        );
    }
    #[test]
    fn native_pdh_second_sample_contains_physical_disk_rates() {
        let mut collector = PlatformCollector::new();
        let mut first = MonitoringData::default();
        collector.collect(Instant::now(), &[], &mut first);
        assert!(
            !first.disk_io.is_empty(),
            "PhysicalDisk provider unavailable: {:?}",
            first.capabilities
        );
        assert!(
            first
                .disk_io
                .iter()
                .all(|value| value.read_bytes_per_sec.is_none())
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut second = MonitoringData::default();
        collector.collect(Instant::now(), &[], &mut second);
        assert_eq!(
            second.capabilities["disk_io"].status,
            MetricStatus::Ok,
            "{:?}",
            second.capabilities
        );
        assert!(
            second
                .disk_io
                .iter()
                .all(|value| value.read_bytes_per_sec.is_some())
        );
        assert_eq!(second.capabilities["cpu_times"].status, MetricStatus::Ok);
        assert!(second.cpu_times.iowait_percent.is_none());
    }
}
