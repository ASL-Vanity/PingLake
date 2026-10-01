//! Read-only local process and socket observations.
//!
//! This module deliberately has no dependency on the wire protocol.  The
//! protocol adapter can map these observations after the v2 contract is
//! frozen.  In particular, an incomplete process table is reported as
//! `Unknown`; it is never converted into a healthy zero count.

#![allow(dead_code)]

use std::net::IpAddr;
#[cfg(any(target_os = "linux", test))]
use std::net::{Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservationStatus {
    Ok,
    Unknown,
    PermissionDenied,
    Unsupported,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessObservation {
    pub process_name: String,
    pub instance_count: Option<u32>,
    pub status: ObservationStatus,
    pub healthy: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketProtocol {
    Tcp,
    Udp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketBinding {
    pub protocol: SocketProtocol,
    pub local_address: IpAddr,
    pub port: u16,
    pub namespace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SocketObservation {
    pub protocol: SocketProtocol,
    pub bindings: Vec<SocketBinding>,
    pub status: ObservationStatus,
    pub error: Option<String>,
}

pub fn observe_process_exact(name: &str, expected_count: Option<u32>) -> ProcessObservation {
    let mut result = ProcessObservation {
        process_name: name.to_owned(),
        instance_count: None,
        status: ObservationStatus::Unsupported,
        healthy: None,
        error: None,
    };
    if name.is_empty()
        || name.len() > 256
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        result.status = ObservationStatus::Unavailable;
        result.error = Some("invalid process name".into());
        return result;
    }
    let observed = observe_process_name(name);
    result.status = observed.status;
    result.instance_count = observed.count;
    result.error = observed.error;
    if result.status == ObservationStatus::Ok {
        result.healthy = expected_count.map(|expected| result.instance_count == Some(expected));
    }
    result
}

struct ProcessScan {
    status: ObservationStatus,
    count: Option<u32>,
    error: Option<String>,
}

#[cfg(target_os = "linux")]
fn observe_process_name(name: &str) -> ProcessScan {
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(error) => {
            return ProcessScan {
                status: if error.kind() == std::io::ErrorKind::PermissionDenied {
                    ObservationStatus::PermissionDenied
                } else {
                    ObservationStatus::Unavailable
                },
                count: None,
                error: Some("failed to enumerate /proc".into()),
            };
        }
    };
    let mut count = 0u32;
    let mut inaccessible = false;
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                inaccessible = true;
                continue;
            }
        };
        let pid = entry.file_name();
        if !pid
            .to_string_lossy()
            .bytes()
            .all(|byte| byte.is_ascii_digit())
        {
            continue;
        }
        let proc_path = entry.path();
        let comm = match std::fs::read_to_string(proc_path.join("comm")) {
            Ok(value) => value.trim_end().to_owned(),
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                inaccessible = true;
                continue;
            }
            Err(_) => continue,
        };
        if comm == name {
            count = count.saturating_add(1);
        }
    }
    if inaccessible {
        ProcessScan {
            status: ObservationStatus::Unknown,
            count: None,
            error: Some("process table visibility is incomplete".into()),
        }
    } else {
        ProcessScan {
            status: ObservationStatus::Ok,
            count: Some(count),
            error: None,
        }
    }
}

#[cfg(target_os = "windows")]
fn observe_process_name(name: &str) -> ProcessScan {
    use sysinfo::{ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let count = system
        .processes()
        .values()
        .filter(|process| process.name().to_string_lossy().eq_ignore_ascii_case(name))
        .count();
    ProcessScan {
        status: ObservationStatus::Ok,
        count: Some(count as u32),
        error: None,
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn observe_process_name(_: &str) -> ProcessScan {
    ProcessScan {
        status: ObservationStatus::Unsupported,
        count: None,
        error: Some("process observation is unsupported on this platform".into()),
    }
}

pub fn observe_local_sockets(
    protocol: SocketProtocol,
    address: Option<IpAddr>,
    port: Option<u16>,
) -> SocketObservation {
    #[cfg(target_os = "linux")]
    {
        return observe_proc_sockets(protocol, address, port);
    }
    #[cfg(target_os = "windows")]
    {
        return observe_windows_sockets(protocol, address, port);
    }
    #[allow(unreachable_code)]
    SocketObservation {
        protocol,
        bindings: Vec::new(),
        status: ObservationStatus::Unsupported,
        error: Some("local socket observation is unsupported on this platform".into()),
    }
}

#[cfg(target_os = "linux")]
fn observe_proc_sockets(
    protocol: SocketProtocol,
    address: Option<IpAddr>,
    port: Option<u16>,
) -> SocketObservation {
    let namespace = std::fs::read_link("/proc/self/ns/net")
        .ok()
        .map(|value| value.to_string_lossy().into_owned());
    let (files, tcp) = match protocol {
        SocketProtocol::Tcp => (["/proc/net/tcp", "/proc/net/tcp6"], true),
        SocketProtocol::Udp => (["/proc/net/udp", "/proc/net/udp6"], false),
    };
    let mut bindings = Vec::new();
    let mut denied = false;
    for (index, path) in files.into_iter().enumerate() {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                denied = true;
                continue;
            }
            Err(_) => continue,
        };
        for line in text.lines().skip(1) {
            let Some(binding) = parse_proc_socket_line(line, protocol, index == 1, tcp) else {
                continue;
            };
            if port.is_some_and(|expected| expected != binding.port)
                || address.is_some_and(|expected| expected != binding.local_address)
            {
                continue;
            }
            bindings.push(SocketBinding {
                namespace: namespace.clone(),
                ..binding
            });
        }
    }
    SocketObservation {
        protocol,
        bindings,
        status: if denied {
            ObservationStatus::Unknown
        } else {
            ObservationStatus::Ok
        },
        error: denied.then(|| "socket table visibility is incomplete".into()),
    }
}

#[cfg(any(target_os = "linux", test))]
fn parse_proc_socket_line(
    line: &str,
    protocol: SocketProtocol,
    v6: bool,
    tcp: bool,
) -> Option<SocketBinding> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() < 4 {
        return None;
    }
    if tcp && fields[3] != "0A" {
        return None;
    }
    let (hex_address, hex_port) = fields[1].split_once(':')?;
    let port = u16::from_str_radix(hex_port, 16).ok()?;
    let local_address = if v6 {
        let raw = u128::from_str_radix(hex_address, 16).ok()?;
        let bytes = raw.to_le_bytes();
        IpAddr::V6(Ipv6Addr::from(bytes))
    } else {
        let raw = u32::from_str_radix(hex_address, 16).ok()?;
        IpAddr::V4(Ipv4Addr::from(raw.to_le_bytes()))
    };
    Some(SocketBinding {
        protocol,
        local_address,
        port,
        namespace: None,
    })
}

#[cfg(target_os = "windows")]
fn observe_windows_sockets(
    protocol: SocketProtocol,
    address: Option<IpAddr>,
    port: Option<u16>,
) -> SocketObservation {
    // The Windows adapter is intentionally isolated here so its IP Helper
    // implementation can be switched to the finalized wire contract. Query
    // failures are explicit and never represented as an empty healthy table.
    let _ = (address, port);
    SocketObservation {
        protocol,
        bindings: Vec::new(),
        status: ObservationStatus::Unsupported,
        error: Some("IP Helper adapter pending contract integration".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_process_name_is_not_a_zero_count() {
        let value = observe_process_exact("bad/name", Some(0));
        assert_eq!(value.status, ObservationStatus::Unavailable);
        assert_eq!(value.instance_count, None);
        assert_eq!(value.healthy, None);
    }

    #[test]
    fn proc_parser_distinguishes_tcp_listeners_and_udp_bindings() {
        let tcp = "0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0        0 0 12345 1 ffff";
        let udp = "0: 0100007F:0035 00000000:0000 07 00000000:00000000 00:00000000 00000000 0        0 0 12345 1 ffff";
        let tcp_binding = parse_proc_socket_line(tcp, SocketProtocol::Tcp, false, true).unwrap();
        assert_eq!(
            tcp_binding.local_address,
            "127.0.0.1".parse::<IpAddr>().unwrap()
        );
        assert_eq!(tcp_binding.port, 8080);
        assert!(parse_proc_socket_line(udp, SocketProtocol::Tcp, false, true).is_none());
        assert!(parse_proc_socket_line(udp, SocketProtocol::Udp, false, false).is_some());
    }
}
