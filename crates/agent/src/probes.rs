use std::{
    net::{IpAddr, SocketAddr},
    time::{Duration, Instant},
};

use chrono::Utc;
use pinglake_protocol::{ProbeKind, ProbeResult, ProbeStatus, ProbeTarget};
use url::Url;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default)]
pub struct ProbePolicy {
    pub allow_private: bool,
    pub allow_loopback: bool,
}

pub fn allowed_address(ip: IpAddr, policy: ProbePolicy) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, _, _] = ip.octets();
            if ip.is_loopback() {
                return policy.allow_loopback;
            }
            if ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_link_local()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (b == 18 || b == 19))
            {
                return false;
            }
            if ip.is_private() {
                return policy.allow_private;
            }
            !ip.is_documentation()
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return allowed_address(v4.into(), policy);
            }
            if ip.is_loopback() {
                return policy.allow_loopback;
            }
            if ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unicast_link_local()
                || (ip.segments()[0] == 0x2001 && ip.segments()[1] == 0xdb8)
            {
                return false;
            }
            if ip == "fd00:ec2::254".parse::<std::net::Ipv6Addr>().unwrap() {
                return false;
            }
            if ip.is_unique_local() {
                return policy.allow_private;
            }
            // Only globally routed IPv6 unicast is an implicit public target.
            ip.segments()[0] & 0xe000 == 0x2000
        }
    }
}

pub fn validate_target(target: &ProbeTarget) -> Result<(), &'static str> {
    if target.name.trim().is_empty()
        || target.name.len() > 128
        || target.target.len() > 2048
        || target.interval_secs < 10
        || target.interval_secs > 86400
        || target.timeout_ms == 0
        || target.timeout_ms > 10000
        || target.timeout_ms >= target.interval_secs * 1000
    {
        return Err("invalid probe limits");
    }
    if target
        .response_contains
        .as_ref()
        .is_some_and(|s| s.len() > 1024)
    {
        return Err("response matcher too long");
    }
    if target.kind == ProbeKind::Http {
        let url = Url::parse(&target.target).map_err(|_| "invalid HTTP URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || target
                .expected_status
                .is_some_and(|s| !(100..=599).contains(&s))
        {
            return Err("invalid HTTP target");
        }
    } else if target.target.trim().is_empty()
        || target.target.contains(['/', '@', '?', '#', '\\'])
        || target.target.chars().any(char::is_whitespace)
        || (target.kind == ProbeKind::Tcp && target.port.unwrap_or(0) == 0)
    {
        return Err("invalid host or port");
    }
    Ok(())
}

pub async fn run_probe(target: ProbeTarget, revision: u64, policy: ProbePolicy) -> ProbeResult {
    let scheduled_at = Utc::now();
    let mut result = ProbeResult {
        sample_id: Uuid::new_v4(),
        target_id: target.id,
        config_revision: revision,
        kind: target.kind,
        scheduled_at,
        completed_at: scheduled_at,
        status: ProbeStatus::Failure,
        latency_ms: None,
        http_status: None,
        error: None,
    };
    if let Err(error) = validate_target(&target) {
        result.status = ProbeStatus::PolicyDenied;
        result.error = Some(error.into());
        return result;
    }
    let duration = Duration::from_millis(target.timeout_ms);
    match tokio::time::timeout(duration, execute(&target, policy)).await {
        Ok(Ok((latency, status))) => {
            result.status = ProbeStatus::Success;
            result.latency_ms = Some(latency);
            result.http_status = status;
        }
        Ok(Err((status, error, http_status))) => {
            result.status = status;
            result.error = Some(error);
            result.http_status = http_status;
        }
        Err(_) => {
            result.status = ProbeStatus::Timeout;
            result.error = Some("probe timeout".into());
        }
    }
    result.completed_at = Utc::now();
    result
}

type ProbeError = (ProbeStatus, String, Option<u16>);
fn failure(message: &str) -> ProbeError {
    (ProbeStatus::Failure, message.into(), None)
}

async fn execute(
    target: &ProbeTarget,
    policy: ProbePolicy,
) -> Result<(f64, Option<u16>), ProbeError> {
    let url = if target.kind == ProbeKind::Http {
        Some(Url::parse(&target.target).map_err(|_| failure("invalid URL"))?)
    } else {
        None
    };
    let host = url
        .as_ref()
        .and_then(Url::host_str)
        .unwrap_or(&target.target);
    let host = host.trim_matches(['[', ']']);
    let port = url
        .as_ref()
        .and_then(Url::port_or_known_default)
        .unwrap_or(target.port.unwrap_or(0));
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| failure("DNS resolution failed"))?
        .take(33)
        .collect();
    if addresses.is_empty() {
        return Err(failure("DNS returned no addresses"));
    }
    if addresses.len() > 32
        || addresses
            .iter()
            .any(|address| !allowed_address(address.ip(), policy))
    {
        return Err((
            ProbeStatus::PolicyDenied,
            "target address denied by local probe policy".into(),
            None,
        ));
    }
    match target.kind {
        ProbeKind::Tcp => {
            let started = Instant::now();
            tokio::net::TcpStream::connect(addresses.as_slice())
                .await
                .map_err(|_| failure("TCP connection failed"))?;
            Ok((started.elapsed().as_secs_f64() * 1000., None))
        }
        ProbeKind::Http => {
            let url = url.as_ref().unwrap();
            // Pin DNS answers so a second resolution cannot bypass the target policy.
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .resolve_to_addrs(host, &addresses)
                .timeout(Duration::from_millis(target.timeout_ms))
                .build()
                .map_err(|_| failure("HTTP client initialization failed"))?;
            let started = Instant::now();
            let mut response = client.get(url.clone()).send().await.map_err(|error| {
                if error.is_timeout() {
                    (ProbeStatus::Timeout, "HTTP timeout".into(), None)
                } else {
                    failure("HTTP connection or TLS validation failed")
                }
            })?;
            let status = response.status().as_u16();
            let expected = target
                .expected_status
                .map_or((200..300).contains(&status), |expected| status == expected);
            if !expected {
                return Err((
                    ProbeStatus::Failure,
                    "unexpected HTTP status".into(),
                    Some(status),
                ));
            }
            if let Some(matcher) = &target.response_contains {
                let mut body = Vec::new();
                while let Some(chunk) = response
                    .chunk()
                    .await
                    .map_err(|_| failure("HTTP body read failed"))?
                {
                    if body.len() + chunk.len() > 64 * 1024 {
                        return Err((
                            ProbeStatus::Failure,
                            "HTTP response exceeds 64 KiB matcher limit".into(),
                            Some(status),
                        ));
                    }
                    body.extend_from_slice(&chunk);
                }
                if !String::from_utf8_lossy(&body).contains(matcher) {
                    return Err((
                        ProbeStatus::Failure,
                        "HTTP response did not match".into(),
                        Some(status),
                    ));
                }
            }
            Ok((started.elapsed().as_secs_f64() * 1000., Some(status)))
        }
        ProbeKind::Icmp => icmp(addresses[0].ip(), target.timeout_ms)
            .await
            .map(|ms| (ms, None)),
        ProbeKind::Dns | ProbeKind::Process | ProbeKind::LocalPort => Err((
            ProbeStatus::Unsupported,
            "probe kind is not implemented by this Agent".into(),
            None,
        )),
    }
}

#[cfg(not(windows))]
async fn icmp(ip: IpAddr, timeout_ms: u64) -> Result<f64, ProbeError> {
    use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence, SurgeError};
    let config = Config::builder()
        .kind(if ip.is_ipv6() { ICMP::V6 } else { ICMP::V4 })
        .build();
    let client = Client::new(&config).map_err(|error| {
        if error.kind() == std::io::ErrorKind::PermissionDenied {
            (
                ProbeStatus::PermissionDenied,
                "ICMP requires socket permission".into(),
                None,
            )
        } else {
            failure("ICMP socket unavailable")
        }
    })?;
    let mut pinger = client.pinger(ip, PingIdentifier(rand::random())).await;
    pinger.timeout(Duration::from_millis(timeout_ms));
    pinger
        .ping(PingSequence(0), &[0; 16])
        .await
        .map(|(_, elapsed)| elapsed.as_secs_f64() * 1000.)
        .map_err(|error| {
            if matches!(error, SurgeError::Timeout { .. }) {
                (ProbeStatus::Timeout, "ICMP timeout".into(), None)
            } else {
                failure("ICMP failed")
            }
        })
}

#[cfg(windows)]
async fn icmp(ip: IpAddr, timeout_ms: u64) -> Result<f64, ProbeError> {
    tokio::task::spawn_blocking(move || native_icmp(ip, timeout_ms))
        .await
        .map_err(|_| failure("ICMP worker stopped"))?
}

#[cfg(windows)]
fn native_icmp(ip: IpAddr, timeout_ms: u64) -> Result<f64, ProbeError> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, INVALID_HANDLE_VALUE},
        NetworkManagement::IpHelper::*,
        Networking::WinSock::*,
    };
    // Reply buffers must be correctly aligned for native reply structures.
    let mut reply = [0_u64; 128];
    let payload = [0_u8; 16];
    unsafe {
        let handle = if ip.is_ipv4() {
            IcmpCreateFile()
        } else {
            Icmp6CreateFile()
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err((
                ProbeStatus::PermissionDenied,
                "ICMP handle unavailable".into(),
                None,
            ));
        }
        let (count, status, rtt) = match ip {
            IpAddr::V4(ip) => {
                let count = IcmpSendEcho(
                    handle,
                    u32::from_ne_bytes(ip.octets()),
                    payload.as_ptr().cast(),
                    payload.len() as u16,
                    std::ptr::null(),
                    reply.as_mut_ptr().cast(),
                    std::mem::size_of_val(&reply) as u32,
                    timeout_ms as u32,
                );
                let result = &*reply.as_ptr().cast::<ICMP_ECHO_REPLY>();
                (count, result.Status, result.RoundTripTime)
            }
            IpAddr::V6(ip) => {
                let source = SOCKADDR_IN6 {
                    sin6_family: AF_INET6,
                    ..Default::default()
                };
                let mut destination = source;
                destination.sin6_addr.u.Byte = ip.octets();
                let count = Icmp6SendEcho2(
                    handle,
                    std::ptr::null_mut(),
                    None,
                    std::ptr::null(),
                    &source,
                    &destination,
                    payload.as_ptr().cast(),
                    payload.len() as u16,
                    std::ptr::null(),
                    reply.as_mut_ptr().cast(),
                    std::mem::size_of_val(&reply) as u32,
                    timeout_ms as u32,
                );
                let result = &*reply.as_ptr().cast::<ICMPV6_ECHO_REPLY_LH>();
                (count, result.Status, result.RoundTripTime)
            }
        };
        let error = GetLastError();
        IcmpCloseHandle(handle);
        if count > 0 && status == 0 {
            Ok(f64::from(rtt))
        } else if error == 11010 || status == 11010 {
            Err((ProbeStatus::Timeout, "ICMP timeout".into(), None))
        } else if error == 5 {
            Err((
                ProbeStatus::PermissionDenied,
                "ICMP access denied".into(),
                None,
            ))
        } else {
            Err(failure("ICMP destination unreachable"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(kind: ProbeKind, address: String, port: Option<u16>) -> ProbeTarget {
        ProbeTarget {
            id: Uuid::new_v4(),
            name: "test".into(),
            kind,
            target: address,
            port,
            enabled: true,
            interval_secs: 30,
            timeout_ms: 200,
            expected_status: None,
            response_contains: None,
        }
    }
    #[test]
    fn policy_rejects_metadata_mapped_ipv6_and_loopback_by_default() {
        for ip in [
            "169.254.169.254",
            "127.0.0.1",
            "10.0.0.1",
            "::ffff:127.0.0.1",
            "fe80::1",
            "100.100.100.200",
        ] {
            assert!(
                !allowed_address(ip.parse().unwrap(), ProbePolicy::default()),
                "{ip}"
            );
        }
        assert!(allowed_address(
            "127.0.0.1".parse().unwrap(),
            ProbePolicy {
                allow_loopback: true,
                ..Default::default()
            }
        ));
        assert!(!allowed_address(
            "169.254.169.254".parse().unwrap(),
            ProbePolicy {
                allow_private: true,
                allow_loopback: true
            }
        ));
    }
    #[tokio::test]
    async fn tcp_probe_and_policy_use_controlled_loopback_target() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let probe = target(
            ProbeKind::Tcp,
            "127.0.0.1".into(),
            Some(listener.local_addr().unwrap().port()),
        );
        assert_eq!(
            run_probe(probe.clone(), 1, ProbePolicy::default())
                .await
                .status,
            ProbeStatus::PolicyDenied
        );
        assert_eq!(
            run_probe(
                probe,
                1,
                ProbePolicy {
                    allow_loopback: true,
                    ..Default::default()
                }
            )
            .await
            .status,
            ProbeStatus::Success
        );
    }
    #[tokio::test]
    async fn http_checks_match_and_do_not_follow_redirects() {
        let app = axum::Router::new()
            .route("/ok", axum::routing::get(|| async { "healthy" }))
            .route(
                "/redirect",
                axum::routing::get(|| async {
                    axum::response::Redirect::temporary("http://169.254.169.254/")
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let policy = ProbePolicy {
            allow_loopback: true,
            ..Default::default()
        };
        let mut probe = target(ProbeKind::Http, format!("{base}/ok"), None);
        probe.response_contains = Some("healthy".into());
        assert_eq!(
            run_probe(probe, 1, policy).await.status,
            ProbeStatus::Success
        );
        assert_eq!(
            run_probe(
                target(ProbeKind::Http, format!("{base}/redirect"), None),
                1,
                policy
            )
            .await
            .status,
            ProbeStatus::Failure
        );
        handle.abort();
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn native_icmp_loopback() {
        assert_eq!(
            run_probe(
                target(ProbeKind::Icmp, "127.0.0.1".into(), None),
                1,
                ProbePolicy {
                    allow_loopback: true,
                    ..Default::default()
                }
            )
            .await
            .status,
            ProbeStatus::Success
        );
    }
}
