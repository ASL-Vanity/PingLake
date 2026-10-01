use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use chrono::Utc;
use pinglake_protocol::{
    AgentHealth, Capability, MetricReport, MetricStatus, MonitoringData, NodeMonitoringConfig,
    ProbeResult, ProbeTarget, ServiceResult,
};
use tokio::{sync::Notify, task::JoinSet};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    client::{ApiClient, SendError},
    metrics::MetricCollector,
    probes::{ProbePolicy, run_probe, validate_target},
    state::AgentState,
};

const QUEUE_CAPACITY: usize = 64;
// Reserve room for sample_age_ms to grow while the report waits for upload.
const MAX_REPORT_BYTES: usize = 256 * 1024 - 1024;
const MAX_SAMPLE_AGE: Duration = Duration::from_secs(300);

struct QueuedReport {
    report: MetricReport,
    collected: Instant,
}

#[derive(Default)]
struct Shared {
    health: AgentHealth,
    outcomes: VecDeque<bool>,
    queue: VecDeque<QueuedReport>,
    config: NodeMonitoringConfig,
    services: Vec<ServiceResult>,
    probes: VecDeque<ProbeResult>,
    endpoint: Capability,
}

impl Shared {
    fn push(&mut self, report: MetricReport, collected: Instant) {
        if self.queue.len() == QUEUE_CAPACITY {
            self.queue.pop_front();
            self.health.dropped_reports += 1;
        }
        self.queue.push_back(QueuedReport { report, collected });
        self.health.queue_length = self.queue.len();
    }
    fn outcome(&mut self, success: bool, duration: Duration, error: Option<String>) {
        self.health.upload_attempts += 1;
        self.health.send_duration_ms = Some(duration.as_secs_f64() * 1000.);
        if success {
            self.health.upload_successes += 1;
            self.health.consecutive_failures = 0;
            self.health.last_success_at = Some(Utc::now());
        } else {
            self.health.upload_failures += 1;
            self.health.consecutive_failures += 1;
        }
        self.health.last_error = error;
        self.outcomes.push_back(success);
        if self.outcomes.len() > 100 {
            self.outcomes.pop_front();
        }
        self.health.success_rate_percent = Some(
            self.outcomes.iter().filter(|outcome| **outcome).count() as f64 * 100.
                / self.outcomes.len() as f64,
        );
    }
}

pub async fn run(
    client: ApiClient,
    state: AgentState,
    collector: MetricCollector,
    interval: u64,
    policy: ProbePolicy,
    endpoint: Option<(std::net::SocketAddr, String)>,
    shutdown: CancellationToken,
) -> Result<()> {
    let shared = Arc::new(Mutex::new(Shared::default()));
    let changed = Arc::new(Notify::new());
    let endpoint_shared = shared.clone();
    let endpoint_loop = async {
        if let Some((bind, origin)) = endpoint {
            let operation = async {
                let listener = tokio::net::TcpListener::bind(bind).await?;
                endpoint_shared.lock().unwrap().endpoint = Capability {
                    status: MetricStatus::Ok,
                    source: "loopback_HTTP".into(),
                    error: None,
                };
                crate::latency_endpoint::serve(listener, origin, shutdown.clone()).await
            };
            if let Err(error) = operation.await {
                tracing::warn!(reason = %error, "browser latency endpoint unavailable; host collection continues");
                endpoint_shared.lock().unwrap().endpoint = Capability {
                    status: MetricStatus::Unavailable,
                    source: "loopback_HTTP".into(),
                    error: Some("latency listener could not start or stopped".into()),
                };
            }
        }
        shutdown.cancelled().await;
        Ok::<(), anyhow::Error>(())
    };
    let result = tokio::try_join!(
        collect_loop(
            collector,
            interval,
            shared.clone(),
            changed.clone(),
            shutdown.clone()
        ),
        send_loop(
            client.clone(),
            state.clone(),
            shared.clone(),
            changed,
            shutdown.clone()
        ),
        config_loop(client, state, shared.clone(), shutdown.clone()),
        checks_loop(shared, policy, shutdown.clone()),
        endpoint_loop,
    );
    shutdown.cancel();
    result.map(|_| ())
}

async fn collect_loop(
    mut collector: MetricCollector,
    interval: u64,
    shared: Arc<Mutex<Shared>>,
    changed: Arc<Notify>,
    shutdown: CancellationToken,
) -> Result<()> {
    let session_id = Uuid::new_v4();
    let mut sequence = 0_u64;
    let mut timer = tokio::time::interval(Duration::from_secs(interval));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = shutdown.cancelled() => return Ok(()), _ = timer.tick() => {} }
        let hub_duration = shared.lock().unwrap().health.send_duration_ms;
        let started = Instant::now();
        let task = tokio::task::spawn_blocking(move || {
            if let Some(ms) = hub_duration {
                collector.record_hub_latency(Duration::from_secs_f64(ms / 1000.));
            }
            let report = collector.collect();
            (collector, report)
        });
        let (returned, mut report) = task.await?;
        collector = returned;
        sequence += 1;
        let mut shared = shared.lock().unwrap();
        let cutoff = Utc::now() - chrono::Duration::days(6);
        shared.probes.retain(|probe| probe.scheduled_at >= cutoff);
        shared.health.collection_duration_ms = started.elapsed().as_secs_f64() * 1000.;
        let data = report
            .monitoring
            .get_or_insert_with(MonitoringData::default);
        data.schema_version = 1;
        data.session_id = session_id;
        data.sample_sequence = sequence;
        data.report_interval_secs = interval;
        data.agent = shared.health.clone();
        data.services = shared.services.clone();
        data.probes = shared.probes.iter().cloned().collect();
        data.capabilities
            .insert("browser_endpoint".into(), shared.endpoint.clone());
        fit_report_budget(&mut report)?;
        shared.push(report, started);
        changed.notify_one();
    }
}

async fn send_loop(
    client: ApiClient,
    state: AgentState,
    shared: Arc<Mutex<Shared>>,
    changed: Arc<Notify>,
    shutdown: CancellationToken,
) -> Result<()> {
    loop {
        // Construct the notification before checking the queue to avoid missed wakeups.
        let notified = changed.notified();
        let next = {
            let mut shared = shared.lock().unwrap();
            let next = shared.queue.pop_front();
            shared.health.queue_length = shared.queue.len();
            next
        };
        let Some(mut next) = next else {
            tokio::select! { _ = shutdown.cancelled() => return Ok(()), _ = notified => {} }
            continue;
        };
        let mut backoff = Duration::from_secs(1);
        let mut retry = false;
        loop {
            if next.collected.elapsed() > MAX_SAMPLE_AGE {
                shared.lock().unwrap().health.dropped_reports += 1;
                break;
            }
            if let Some(data) = &mut next.report.monitoring {
                data.agent.sample_age_ms = next.collected.elapsed().as_secs_f64() * 1000.;
            }
            if retry {
                shared.lock().unwrap().health.retries += 1;
            }
            let started = Instant::now();
            let result = tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                result = client.send_metrics(state.agent_id, &state.agent_secret, &next.report) => result,
            };
            shared.lock().unwrap().outcome(
                result.is_ok(),
                started.elapsed(),
                result.as_ref().err().map(ToString::to_string),
            );
            match result {
                Ok(()) => break,
                Err(SendError::Unauthorized) => {
                    bail!("agent authentication was rejected by the hub")
                }
                Err(SendError::Permanent(status)) => {
                    bail!("metric report was rejected with HTTP {status}")
                }
                Err(SendError::Transient(_)) => {}
            }
            retry = true;
            tokio::select! { _ = shutdown.cancelled() => return Ok(()), _ = tokio::time::sleep(backoff) => {} }
            backoff = backoff.saturating_mul(2).min(Duration::from_secs(60));
        }
    }
}

fn validate_config(config: &NodeMonitoringConfig) -> Result<()> {
    if config.services.len() > 32 || config.probes.len() > 32 {
        bail!("monitor configuration exceeds target limits");
    }
    let mut ids = HashSet::new();
    for service in &config.services {
        if service.id.is_nil()
            || !ids.insert(service.id)
            || service.name.trim().is_empty()
            || service.name.len() > 256
            || service.name.starts_with('-')
            || service
                .name
                .chars()
                .any(|c| !c.is_ascii_alphanumeric() && !"_.@- ".contains(c))
            || !matches!(service.expected_state.as_str(), "running" | "stopped")
        {
            bail!("invalid service configuration");
        }
    }
    ids.clear();
    for probe in &config.probes {
        if !ids.insert(probe.id) {
            bail!("duplicate probe ID");
        }
        validate_target(probe).map_err(anyhow::Error::msg)?;
    }
    Ok(())
}

async fn config_loop(
    client: ApiClient,
    state: AgentState,
    shared: Arc<Mutex<Shared>>,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_secs(15));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! { _ = shutdown.cancelled() => return Ok(()), _ = timer.tick() => {} }
        let fetched = tokio::select! {
            _ = shutdown.cancelled() => return Ok(()),
            fetched = client.monitoring_config(state.agent_id, &state.agent_secret) => fetched,
        };
        match fetched {
            Ok(config) => {
                let mut shared = shared.lock().unwrap();
                if let Err(error) = validate_config(&config) {
                    shared.health.config_error = Some(error.to_string());
                } else {
                    shared.config = config;
                    shared.health.config_error = None;
                }
            }
            Err(SendError::Unauthorized) => {
                bail!("monitoring configuration authentication rejected")
            }
            Err(error) => shared.lock().unwrap().health.config_error = Some(error.to_string()),
        }
    }
}

enum CheckResult {
    Probe(ProbeResult),
    Services(u64, Vec<ServiceResult>),
}

async fn checks_loop(
    shared: Arc<Mutex<Shared>>,
    policy: ProbePolicy,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut tasks = JoinSet::new();
    let mut next = HashMap::<Uuid, Instant>::new();
    let mut running = HashSet::new();
    let mut revision = None;
    let mut services_at = Instant::now();
    let mut services_running = false;
    let mut schedule_origin = Instant::now();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => { tasks.abort_all(); return Ok(()); }
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Ok(result) = result {
                    let mut shared = shared.lock().unwrap();
                    match result {
                        CheckResult::Probe(probe) => {
                            if Some(probe.config_revision) == revision {
                                running.remove(&probe.target_id);
                                if shared.probes.len() == 64 { shared.probes.pop_front(); }
                                shared.probes.push_back(probe);
                            }
                        }
                        CheckResult::Services(version, services) => {
                            if Some(version) == revision { services_running = false; shared.services = services; }
                        }
                    }
                }
            }
            _ = timer.tick() => {
                let config = shared.lock().unwrap().config.clone();
                if revision != Some(config.revision) {
                    tasks.abort_all(); next.clear(); running.clear(); services_running = false;
                    services_at = Instant::now(); revision = Some(config.revision);
                    schedule_origin = Instant::now();
                    let mut shared = shared.lock().unwrap();
                    shared.services.clear(); shared.probes.clear();
                    shared.health.applied_config_revision = revision;
                }
                if !services_running && Instant::now() >= services_at {
                    services_running = true; services_at = Instant::now() + Duration::from_secs(30);
                    let services_config = config.clone();
                    tasks.spawn(async move { CheckResult::Services(services_config.revision, crate::services::check_services(&services_config).await) });
                }
                for target in due_probes(&config, &next, &running, schedule_origin, Instant::now()) {
                    running.insert(target.id);
                    next.insert(target.id, Instant::now() + Duration::from_secs(target.interval_secs));
                    let version = config.revision;
                    tasks.spawn(async move { CheckResult::Probe(run_probe(target, version, policy).await) });
                }
            }
        }
    }
}

fn due_probes(
    config: &NodeMonitoringConfig,
    next: &HashMap<Uuid, Instant>,
    running: &HashSet<Uuid>,
    origin: Instant,
    now: Instant,
) -> Vec<ProbeTarget> {
    let mut due = config
        .probes
        .iter()
        .filter(|target| {
            target.enabled
                && !running.contains(&target.id)
                && next.get(&target.id).is_none_or(|next| *next <= now)
        })
        .collect::<Vec<_>>();
    due.sort_by_key(|target| next.get(&target.id).copied().unwrap_or(origin));
    due.into_iter()
        .take(4_usize.saturating_sub(running.len()))
        .cloned()
        .collect()
}

fn fit_report_budget(report: &mut MetricReport) -> Result<()> {
    while serde_json::to_vec(report)?.len() > MAX_REPORT_BYTES {
        let data = report
            .monitoring
            .get_or_insert_with(MonitoringData::default);
        data.capabilities.insert(
            "report_budget".into(),
            Capability {
                status: MetricStatus::Unavailable,
                source: "agent".into(),
                error: Some("detailed metrics truncated to fit 256 KiB report limit".into()),
            },
        );
        if !data.probes.is_empty() {
            data.probes.drain(..data.probes.len().div_ceil(2));
        } else if !data.services.is_empty() {
            data.services.truncate(data.services.len() / 2);
        } else if !data.network_health.is_empty() {
            data.network_health.truncate(data.network_health.len() / 2);
        } else if !data.disk_io.is_empty() {
            data.disk_io.truncate(data.disk_io.len() / 2);
        } else if !data.inodes.is_empty() {
            data.inodes.truncate(data.inodes.len() / 2);
        } else if !data.cpu_cores.is_empty() {
            data.cpu_cores.truncate(data.cpu_cores.len() / 2);
        } else if !report.processes.is_empty() {
            report.processes.truncate(report.processes.len() / 2);
        } else if !report.interfaces.is_empty() {
            report.interfaces.truncate(report.interfaces.len() / 2);
        } else if !report.disks.is_empty() {
            report.disks.truncate(report.disks.len() / 2);
        } else {
            bail!("base report exceeds size limit");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queue_is_bounded_and_drops_oldest_not_newest() {
        let mut shared = Shared::default();
        let mut collector = MetricCollector::new();
        let report = collector.collect();
        for sequence in 1..=70 {
            let mut report = report.clone();
            report.monitoring.as_mut().unwrap().sample_sequence = sequence;
            shared.push(report, Instant::now());
        }
        assert_eq!(shared.queue.len(), QUEUE_CAPACITY);
        assert_eq!(shared.health.dropped_reports, 6);
        assert_eq!(
            shared
                .queue
                .front()
                .unwrap()
                .report
                .monitoring
                .as_ref()
                .unwrap()
                .sample_sequence,
            7
        );
    }
    #[test]
    fn success_rate_uses_last_hundred_real_upload_attempts() {
        let mut shared = Shared::default();
        for _ in 0..100 {
            shared.outcome(false, Duration::from_millis(1), Some("timeout".into()));
        }
        for _ in 0..50 {
            shared.outcome(true, Duration::from_millis(1), None);
        }
        assert_eq!(shared.health.success_rate_percent, Some(50.));
        assert_eq!(shared.health.upload_attempts, 150);
        assert_eq!(shared.health.consecutive_failures, 0);
    }
    #[test]
    fn oversized_details_preserve_a_reportable_base_snapshot() {
        let mut report = MetricCollector::new().collect();
        report.monitoring.as_mut().unwrap().cpu_cores = (0..1024)
            .map(|index| pinglake_protocol::CpuCore {
                id: format!("{index}-{}", "x".repeat(256)),
                usage_percent: 1.,
                frequency_mhz: None,
            })
            .collect();
        fit_report_budget(&mut report).unwrap();
        assert!(serde_json::to_vec(&report).unwrap().len() <= MAX_REPORT_BYTES);
        assert_eq!(
            report.monitoring.as_ref().unwrap().capabilities["report_budget"].status,
            MetricStatus::Unavailable
        );
        assert!(report.memory_total_bytes > 0);
    }
    #[test]
    fn never_run_targets_precede_slow_targets_that_are_due_again() {
        let config = NodeMonitoringConfig {
            probes: (0..6)
                .map(|_| ProbeTarget {
                    id: Uuid::new_v4(),
                    name: "slow target".into(),
                    kind: pinglake_protocol::ProbeKind::Tcp,
                    target: "example.com".into(),
                    port: Some(443),
                    enabled: true,
                    interval_secs: 10,
                    timeout_ms: 9999,
                    expected_status: None,
                    response_contains: None,
                })
                .collect(),
            ..Default::default()
        };
        let origin = Instant::now();
        let next = config.probes[..4]
            .iter()
            .map(|target| (target.id, origin + Duration::from_secs(10)))
            .collect();
        let due = due_probes(
            &config,
            &next,
            &HashSet::new(),
            origin,
            origin + Duration::from_secs(20),
        );
        assert_eq!(due.len(), 4);
        assert_eq!(due[0].id, config.probes[4].id);
        assert_eq!(due[1].id, config.probes[5].id);
    }
    #[tokio::test]
    async fn occupied_optional_endpoint_does_not_stop_host_uploads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let received = Arc::new(AtomicUsize::new(0));
        let observed = received.clone();
        let app = axum::Router::new()
            .route(
                "/api/v1/agent/config",
                axum::routing::get(|| async { axum::Json(NodeMonitoringConfig::default()) }),
            )
            .route(
                "/api/v1/agent/metrics",
                axum::routing::post(move |axum::Json(report): axum::Json<MetricReport>| {
                    let observed = observed.clone();
                    async move {
                        if report.monitoring.as_ref().unwrap().capabilities["browser_endpoint"]
                            .status
                            == MetricStatus::Unavailable
                        {
                            observed.fetch_add(1, Ordering::Relaxed);
                        }
                        axum::http::StatusCode::NO_CONTENT
                    }
                }),
            );
        let hub_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = ApiClient::new(
            url::Url::parse(&format!("http://{}", hub_listener.local_addr().unwrap())).unwrap(),
            false,
        )
        .unwrap();
        let hub = tokio::spawn(async move {
            axum::serve(hub_listener, app).await.unwrap();
        });
        let occupied = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let shutdown = CancellationToken::new();
        let worker = tokio::spawn(run(
            client,
            AgentState {
                version: 1,
                agent_id: Uuid::new_v4(),
                agent_secret: "test".into(),
            },
            MetricCollector::new(),
            1,
            ProbePolicy::default(),
            Some((
                occupied.local_addr().unwrap(),
                "https://monitor.example.com".into(),
            )),
            shutdown.clone(),
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while received.load(Ordering::Relaxed) < 2 && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        shutdown.cancel();
        worker.await.unwrap().unwrap();
        hub.abort();
        assert!(received.load(Ordering::Relaxed) >= 2);
    }
    #[tokio::test]
    async fn collection_continues_when_uploads_fail() {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let changed = Arc::new(Notify::new());
        let shutdown = CancellationToken::new();
        let client = ApiClient::new(url::Url::parse("http://127.0.0.1:9").unwrap(), false).unwrap();
        let state = AgentState {
            version: 1,
            agent_id: Uuid::new_v4(),
            agent_secret: "test".into(),
        };
        let collect = tokio::spawn(collect_loop(
            MetricCollector::new(),
            1,
            shared.clone(),
            changed.clone(),
            shutdown.clone(),
        ));
        let send = tokio::spawn(send_loop(
            client,
            state,
            shared.clone(),
            changed,
            shutdown.clone(),
        ));
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let ready = {
                let shared = shared.lock().unwrap();
                !shared.queue.is_empty() && shared.health.upload_failures > 0
            };
            if ready || tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let queued = shared.lock().unwrap().queue.len();
        let failures = shared.lock().unwrap().health.upload_failures;
        shutdown.cancel();
        collect.await.unwrap().unwrap();
        send.await.unwrap().unwrap();
        assert!(
            queued >= 1,
            "collector must enqueue additional samples while uploader retries"
        );
        assert!(failures >= 1);
    }
}
