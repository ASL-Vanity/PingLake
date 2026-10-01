use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Result, bail};
use chrono::{DateTime, Utc};
use pinglake_protocol::{
    AgentHealth, Capability, CheckStatus, LocalPortResult, MetricReport, MetricStatus,
    MonitoringData, NodeMonitoringConfig, ProbeKind, ProbeResult, ProbeTarget, ProcessResult,
    ServiceResult,
};
use rand::Rng;
use tokio::{sync::Notify, task::JoinSet};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    client::{ApiClient, SendError},
    metrics::MetricCollector,
    probes::{ProbePolicy, run_probe, validate_target},
    spool::Spool,
    state::AgentState,
};

const QUEUE_CAPACITY: usize = 64;
// Reserve room for sample_age_ms to grow while the report waits for upload.
const MAX_REPORT_BYTES: usize = 256 * 1024 - 1024;
const MAX_SAMPLE_AGE: Duration = Duration::from_secs(300);

#[derive(Clone)]
struct QueuedReport {
    report: MetricReport,
    collected: Instant,
    queued_at: DateTime<Utc>,
}

fn load_queue(path: &Path) -> Result<VecDeque<QueuedReport>> {
    let spool = Spool::open(path, "legacy-agent", "legacy-hub")?;
    let mut queue = VecDeque::new();
    let now = Utc::now();
    for item in spool.restore()? {
        let age = now
            .signed_duration_since(item.queued_at)
            .to_std()
            .unwrap_or_default();
        queue.push_back(QueuedReport {
            report: item.report,
            collected: Instant::now().checked_sub(age).unwrap_or_else(Instant::now),
            queued_at: item.queued_at,
        });
    }
    Ok(queue)
}

fn persist_queue(path: &Path, shared: &Shared, _persist_lock: &Arc<Mutex<()>>) -> Result<()> {
    let spool = Spool::open(path, "legacy-agent", "legacy-hub")?;
    let mut records = Vec::new();
    if let Some(item) = &shared.in_flight {
        records.push((item.report.clone(), item.queued_at));
    }
    records.extend(
        shared
            .queue
            .iter()
            .map(|item| (item.report.clone(), item.queued_at)),
    );
    spool.replace_all(&records)
}
#[derive(Default)]
struct Shared {
    health: AgentHealth,
    outcomes: VecDeque<bool>,
    queue: VecDeque<QueuedReport>,
    in_flight: Option<QueuedReport>,
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
        self.queue.push_back(QueuedReport {
            report,
            collected,
            queued_at: Utc::now(),
        });
        self.health.queue_length = self.queue.len();
    }

    fn drop_report(&mut self) {
        self.health.dropped_reports += 1;
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

#[allow(clippy::too_many_arguments)]
pub async fn run(
    client: ApiClient,
    state: AgentState,
    collector: MetricCollector,
    interval: u64,
    monitoring_schema_max: u32,
    queue_path: PathBuf,
    policy: ProbePolicy,
    endpoint: Option<(std::net::SocketAddr, String)>,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut initial = Shared::default();
    match load_queue(&queue_path) {
        Ok(queue) => {
            initial.queue = queue;
            initial.health.queue_length = initial.queue.len();
        }
        Err(error) => tracing::warn!(reason = %error, "pending report queue could not be restored"),
    }
    let shared = Arc::new(Mutex::new(initial));
    let persist_lock = Arc::new(Mutex::new(()));
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
            monitoring_schema_max,
            queue_path.clone(),
            persist_lock.clone(),
            shared.clone(),
            changed.clone(),
            shutdown.clone()
        ),
        send_loop(
            client.clone(),
            state.clone(),
            monitoring_schema_max,
            shared.clone(),
            changed,
            queue_path,
            persist_lock,
            shutdown.clone()
        ),
        config_loop(client, state, shared.clone(), shutdown.clone()),
        checks_loop(shared, policy, shutdown.clone()),
        endpoint_loop,
    );
    shutdown.cancel();
    result.map(|_| ())
}

#[allow(clippy::too_many_arguments)]
async fn collect_loop(
    mut collector: MetricCollector,
    interval: u64,
    monitoring_schema_max: u32,
    queue_path: PathBuf,
    persist_lock: Arc<Mutex<()>>,
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
        data.schema_version = if monitoring_schema_max >= 2 { 2 } else { 1 };
        data.session_id = session_id;
        data.sample_sequence = sequence;
        data.report_interval_secs = interval;
        data.agent = shared.health.clone();
        data.services = shared.services.clone();
        data.probes = shared.probes.iter().cloned().collect();
        let process_ids = shared
            .config
            .process_checks
            .iter()
            .map(|check| check.id)
            .collect::<HashSet<_>>();
        data.process_checks = shared
            .services
            .iter()
            .filter(|service| process_ids.contains(&service.id))
            .map(|service| ProcessResult {
                id: service.id,
                name: service.name.clone(),
                sample_id: Some(Uuid::new_v4()),
                config_revision: Some(service.config_revision),
                scheduled_at: Some(service.checked_at),
                completed_at: Some(service.checked_at),
                checked_at: Some(service.checked_at),
                healthy: service.healthy,
                status: match service.status {
                    MetricStatus::Ok => CheckStatus::Ok,
                    MetricStatus::PermissionDenied => CheckStatus::PermissionDenied,
                    MetricStatus::Unsupported => CheckStatus::Unsupported,
                    MetricStatus::Stale => CheckStatus::Stale,
                    MetricStatus::WarmingUp => CheckStatus::WarmingUp,
                    MetricStatus::Unavailable => CheckStatus::Unavailable,
                },
                process_name: shared
                    .config
                    .process_checks
                    .iter()
                    .find(|check| check.id == service.id)
                    .map(|check| check.process_name.clone())
                    .unwrap_or_default(),
                count: Some((service.state == "running") as u32),
                expected_count: shared
                    .config
                    .process_checks
                    .iter()
                    .find(|check| check.id == service.id)
                    .and_then(|check| check.expected_count),
                error: service.error.clone(),
                reason: service.error.clone(),
            })
            .collect();
        data.local_port_checks = shared
            .config
            .local_port_checks
            .iter()
            .filter(|check| check.enabled)
            .map(|check| {
                let listening = data.tcp.listening_port_numbers.contains(&check.port);
                LocalPortResult {
                    id: check.id,
                    name: check.name.clone(),
                    sample_id: Some(Uuid::new_v4()),
                    config_revision: Some(shared.config.revision),
                    scheduled_at: Some(report.collected_at),
                    completed_at: Some(report.collected_at),
                    checked_at: Some(report.collected_at),
                    status: CheckStatus::Ok,
                    healthy: Some(listening),
                    address_scope: check.address_scope.clone(),
                    address_family: check.address_family,
                    protocol: check.protocol,
                    observed_addresses: Vec::new(),
                    port: check.port,
                    latency_ms: None,
                    error: if listening {
                        None
                    } else {
                        Some("port is not listening".into())
                    },
                    reason: if listening {
                        None
                    } else {
                        Some("not_listening".into())
                    },
                }
            })
            .collect();
        if data.schema_version < 2 {
            downgrade_monitoring_to_v1(data);
        }
        data.capabilities
            .insert("browser_endpoint".into(), shared.endpoint.clone());
        fit_report_budget(&mut report)?;
        shared.push(report, started);
        if let Err(error) = persist_queue(&queue_path, &shared, &persist_lock) {
            tracing::error!(reason=%error, "spool persistence degraded; collection continues");
        }
        changed.notify_one();
    }
}

fn downgrade_monitoring_to_v1(data: &mut MonitoringData) {
    data.schema_version = 1;
    data.probes.retain(|probe| {
        matches!(
            probe.kind,
            ProbeKind::Icmp | ProbeKind::Tcp | ProbeKind::Http
        )
    });
    data.process_checks.clear();
    data.local_port_checks.clear();
}

async fn send_loop(
    client: ApiClient,
    state: AgentState,
    monitoring_schema_max: u32,
    shared: Arc<Mutex<Shared>>,
    changed: Arc<Notify>,
    queue_path: PathBuf,
    persist_lock: Arc<Mutex<()>>,
    shutdown: CancellationToken,
) -> Result<()> {
    loop {
        // Construct the notification before checking the queue to avoid missed wakeups.
        let notified = changed.notified();
        let next = {
            let mut shared = shared.lock().unwrap();
            let next = shared.queue.pop_front();
            shared.in_flight = next.clone();
            shared.health.queue_length = shared.queue.len();
            if let Err(error) = persist_queue(&queue_path, &shared, &persist_lock) {
                tracing::error!(reason=%error, "spool persistence degraded; collection continues");
            }
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
                let mut shared = shared.lock().unwrap();
                shared.in_flight = None;
                shared.drop_report();
                if let Err(error) = persist_queue(&queue_path, &shared, &persist_lock) {
                    tracing::error!(reason=%error, "spool persistence degraded; collection continues");
                }
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
                _ = shutdown.cancelled() => {
                    let mut shared = shared.lock().unwrap();
                    shared.in_flight = None;
                    shared.push(next.report.clone(), next.collected);
                    if let Err(error) = persist_queue(&queue_path, &shared, &persist_lock) { tracing::error!(reason=%error, "spool persistence degraded; collection continues"); }
                    return Ok(())
                },
                result = client.send_metrics_with_schema(state.agent_id, &state.agent_secret, &next.report, monitoring_schema_max) => result,
            };
            shared.lock().unwrap().outcome(
                result.is_ok(),
                started.elapsed(),
                result.as_ref().err().map(ToString::to_string),
            );
            match result {
                Ok(()) => {
                    let mut shared = shared.lock().unwrap();
                    shared.in_flight = None;
                    if let Err(error) = persist_queue(&queue_path, &shared, &persist_lock) {
                        tracing::error!(reason=%error, "spool persistence degraded; collection continues");
                    }
                    break;
                }
                Err(SendError::Unauthorized) => {
                    bail!("agent authentication was rejected by the hub")
                }
                Err(SendError::Permanent(status)) => {
                    bail!("metric report was rejected with HTTP {status}")
                }
                Err(SendError::Transient(_)) => {}
            }
            retry = true;
            let delay = retry_delay(backoff);
            tokio::select! {
                _ = shutdown.cancelled() => {
                    let mut shared = shared.lock().unwrap();
                    shared.in_flight = None;
                    shared.push(next.report.clone(), next.collected);
                    if let Err(error) = persist_queue(&queue_path, &shared, &persist_lock) { tracing::error!(reason=%error, "spool persistence degraded; collection continues"); }
                    return Ok(())
                }
                _ = tokio::time::sleep(delay) => {}
            }
            backoff = backoff.saturating_mul(2).min(Duration::from_secs(60));
        }
    }
}

fn retry_delay(base: Duration) -> Duration {
    // Keep retries spread over a bounded 75%..125% window so a fleet does not
    // synchronize after a common hub outage. The cap is applied by the caller
    // before this function, and the jitter never turns a retry into a busy loop.
    let percent = rand::rng().random_range(75_u32..=125);
    base.saturating_mul(percent)
        .checked_div(100)
        .unwrap_or(base)
}

fn validate_config(config: &NodeMonitoringConfig) -> Result<()> {
    if config.services.len() > 32
        || config.process_checks.len() > 32
        || config.probes.len() > 32
        || config.local_port_checks.len() > 32
    {
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
    for process in &config.process_checks {
        if process.id.is_nil()
            || !ids.insert(process.id)
            || process.name.trim().is_empty()
            || process.name.len() > 256
            || process.process_name.trim().is_empty()
            || process.process_name.len() > 256
            || !matches!(process.expected_state.as_str(), "running" | "stopped")
            || process
                .process_name
                .chars()
                .any(|c| c == '/' || c == '\\' || c.is_control())
        {
            bail!("invalid process configuration");
        }
    }
    for check in &config.local_port_checks {
        if check.id.is_nil()
            || !ids.insert(check.id)
            || check.name.trim().is_empty()
            || check.name.len() > 128
            || check.port == 0
            || !(10..=86_400).contains(&check.interval_secs)
            || check.timeout_ms == 0
            || check.timeout_ms > 10_000
            || check.timeout_ms >= check.interval_secs * 1000
        {
            bail!("invalid local port check configuration");
        }
    }
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
    let configured = config.probes.clone();
    let mut due = configured
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
    fn pending_queue_survives_a_round_trip() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pending-reports.json");
        let lock = Arc::new(Mutex::new(()));
        let mut shared = Shared::default();
        let mut report = MetricCollector::new().collect();
        report.monitoring.as_mut().unwrap().sample_sequence = 17;
        shared.push(report, Instant::now());
        persist_queue(&path, &shared, &lock).unwrap();

        let restored = load_queue(&path).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(
            restored
                .front()
                .unwrap()
                .report
                .monitoring
                .as_ref()
                .unwrap()
                .sample_sequence,
            17
        );
    }

    #[test]
    fn legacy_schema_downgrade_removes_v2_only_results() {
        let mut data = MonitoringData {
            schema_version: 2,
            probes: vec![
                ProbeResult {
                    sample_id: Uuid::new_v4(),
                    target_id: Uuid::new_v4(),
                    config_revision: 1,
                    kind: ProbeKind::Dns,
                    scheduled_at: Utc::now(),
                    completed_at: Utc::now(),
                    status: pinglake_protocol::ProbeStatus::Success,
                    healthy: Some(true),
                    latency_ms: Some(1.0),
                    http_status: None,
                    error: None,
                    dns: None,
                },
                ProbeResult {
                    sample_id: Uuid::new_v4(),
                    target_id: Uuid::new_v4(),
                    config_revision: 1,
                    kind: ProbeKind::Http,
                    scheduled_at: Utc::now(),
                    completed_at: Utc::now(),
                    status: pinglake_protocol::ProbeStatus::Success,
                    healthy: Some(true),
                    latency_ms: Some(1.0),
                    http_status: Some(200),
                    error: None,
                    dns: None,
                },
            ],
            process_checks: vec![ProcessResult::default()],
            local_port_checks: vec![LocalPortResult::default()],
            ..Default::default()
        };
        downgrade_monitoring_to_v1(&mut data);
        assert_eq!(data.schema_version, 1);
        assert_eq!(data.probes.len(), 1);
        assert_eq!(data.probes[0].kind, ProbeKind::Http);
        assert!(data.process_checks.is_empty());
        assert!(data.local_port_checks.is_empty());
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
    fn retry_delay_stays_within_bounded_jitter_window() {
        let base = Duration::from_secs(8);
        for _ in 0..128 {
            let delay = retry_delay(base);
            assert!(delay >= Duration::from_secs(6));
            assert!(delay <= Duration::from_secs(10));
        }
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
                    dns: None,
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
        let queue_dir = tempfile::tempdir().unwrap();
        let queue_path = queue_dir.path().join("pending-reports.json");
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
            2,
            queue_path,
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
        let queue_dir = tempfile::tempdir().unwrap();
        let queue_path = queue_dir.path().join("pending-reports.json");
        let persist_lock = Arc::new(Mutex::new(()));
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
            2,
            queue_path.clone(),
            persist_lock.clone(),
            shared.clone(),
            changed.clone(),
            shutdown.clone(),
        ));
        let send = tokio::spawn(send_loop(
            client,
            state,
            2,
            shared.clone(),
            changed,
            queue_path,
            persist_lock,
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
