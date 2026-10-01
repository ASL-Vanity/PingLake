use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use pinglake_protocol::{
    MetricReport, MetricStatus, MonitoringData, MonitoringHistoryPoint, NodeMonitoringConfig,
    ProbeKind, ProbeResult, ProbeStatistics, ProbeStatus,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

use crate::{db::Database, error::AppError};

const MAX_HISTORY_POINTS: usize = 240;
pub(crate) const MAX_HISTORY_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
#[error("monitoring configuration revision changed: requested {requested}, current {current}")]
pub(crate) struct MonitoringRevisionConflict {
    requested: u64,
    current: u64,
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_time(value: String) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(&value)?.with_timezone(&Utc))
}

pub(crate) fn config_from(connection: &Connection, node_id: Uuid) -> Result<NodeMonitoringConfig> {
    let value: Option<String> = connection.query_row(
        "SELECT config_json FROM monitoring_configs WHERE node_id = ?1 ORDER BY revision DESC LIMIT 1",
        [node_id.to_string()], |row| row.get(0),
    ).optional()?;
    value
        .map(|value| serde_json::from_str(&value).context("invalid monitoring config"))
        .transpose()
        .map(|config| config.unwrap_or_default())
}

impl Database {
    pub fn monitoring_config(&self, node_id: Uuid) -> Result<Option<NodeMonitoringConfig>> {
        let connection = self.lock()?;
        if !node_exists(&connection, node_id)? {
            return Ok(None);
        }
        Ok(Some(config_from(&connection, node_id)?))
    }

    pub fn agent_monitoring_config(
        &self,
        node_id: Uuid,
        secret_hash: &str,
    ) -> Result<Option<NodeMonitoringConfig>> {
        let connection = self.lock()?;
        let stored: Option<String> = connection
            .query_row(
                "SELECT secret_hash FROM nodes WHERE id = ?1",
                [node_id.to_string()],
                |row| row.get(0),
            )
            .optional()?;
        if !stored.is_some_and(|stored| super::db::constant_time_equal(&stored, secret_hash)) {
            return Ok(None);
        }
        Ok(Some(config_from(&connection, node_id)?))
    }

    pub fn save_monitoring_config(
        &self,
        node_id: Uuid,
        mut config: NodeMonitoringConfig,
    ) -> Result<Option<NodeMonitoringConfig>> {
        let mut connection = self.lock()?;
        let transaction =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if !node_exists(&transaction, node_id)? {
            return Ok(None);
        }
        let current_revision = config_from(&transaction, node_id)?.revision;
        if config.revision != current_revision {
            return Err(MonitoringRevisionConflict {
                requested: config.revision,
                current: current_revision,
            }
            .into());
        }
        config.revision = current_revision
            .checked_add(1)
            .context("monitoring config revision exhausted")?;
        transaction.execute(
            "INSERT INTO monitoring_configs(node_id, revision, effective_at, config_json) VALUES (?1, ?2, ?3, ?4)",
            params![node_id.to_string(), i64::try_from(config.revision)?, timestamp(Utc::now()), serde_json::to_string(&config)?],
        )?;
        transaction.execute(
            "UPDATE nodes SET browser_latency_url = ?2 WHERE id = ?1",
            params![node_id.to_string(), config.browser_latency_url],
        )?;
        transaction.execute(
            "DELETE FROM monitoring_check_state WHERE node_id = ?1",
            [node_id.to_string()],
        )?;
        transaction.commit()?;
        Ok(Some(config))
    }

    pub fn browser_latency_origins(&self) -> Result<Vec<String>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT DISTINCT browser_latency_url FROM nodes WHERE browser_latency_url IS NOT NULL",
        )?;
        let urls = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut origins = urls
            .into_iter()
            .filter_map(|url| reqwest::Url::parse(&url).ok())
            .map(|url| url.origin().ascii_serialization())
            .collect::<Vec<_>>();
        origins.sort();
        origins.dedup();
        Ok(origins)
    }

    pub fn validate_monitoring_identity(
        &self,
        node_id: Uuid,
        data: &MonitoringData,
    ) -> Result<bool> {
        let connection = self.lock()?;
        let mut configs = BTreeMap::new();
        for revision in data
            .services
            .iter()
            .map(|result| result.config_revision)
            .chain(data.probes.iter().map(|result| result.config_revision))
        {
            if configs.contains_key(&revision) {
                continue;
            }
            let json: Option<String> = connection.query_row(
                "SELECT config_json FROM monitoring_configs WHERE node_id = ?1 AND revision = ?2",
                params![node_id.to_string(), i64::try_from(revision)?], |row| row.get(0),
            ).optional()?;
            let Some(json) = json else {
                return Ok(false);
            };
            configs.insert(
                revision,
                serde_json::from_str::<NodeMonitoringConfig>(&json)?,
            );
        }
        for result in &data.services {
            if !configs[&result.config_revision]
                .services
                .iter()
                .any(|check| check.id == result.id && check.enabled && check.name == result.name)
            {
                return Ok(false);
            }
        }
        for result in &data.probes {
            if !configs[&result.config_revision]
                .probes
                .iter()
                .any(|target| {
                    target.id == result.target_id && target.enabled && target.kind == result.kind
                })
            {
                return Ok(false);
            }
            let previous: Option<String> = connection
                .query_row(
                    "SELECT result_json FROM probe_samples WHERE node_id = ?1 AND sample_id = ?2",
                    params![node_id.to_string(), result.sample_id.to_string()],
                    |row| row.get(0),
                )
                .optional()?;
            if previous.is_some_and(|previous| {
                serde_json::to_string(result).is_ok_and(|value| value != previous)
            }) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    pub fn monitoring_history(
        &self,
        node_id: Uuid,
        minutes: u64,
        section: &str,
        device: Option<&str>,
    ) -> Result<Option<Vec<MonitoringHistoryPoint>>> {
        let connection = self.lock()?;
        if !node_exists(&connection, node_id)? {
            return Ok(None);
        }
        let cutoff = timestamp(Utc::now() - chrono::Duration::minutes(minutes as i64));
        let bucket_seconds = (minutes * 60).div_ceil(MAX_HISTORY_POINTS as u64).max(1);
        let mut statement = connection.prepare(
            "SELECT collected_at, received_at, monitoring_json FROM metrics WHERE id IN (
                SELECT MAX(id) FROM metrics WHERE node_id = ?1 AND received_at >= ?2 AND monitoring_json IS NOT NULL
                GROUP BY CAST(unixepoch(received_at) / ?3 AS INTEGER) ORDER BY MAX(id) DESC LIMIT ?4
             ) ORDER BY received_at, id",
        )?;
        let rows = statement.query_map(
            params![
                node_id.to_string(),
                cutoff,
                bucket_seconds,
                MAX_HISTORY_POINTS
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )?;
        let mut points = Vec::new();
        for row in rows {
            let (collected_at, received_at, json) = row?;
            let mut monitoring: MonitoringData = serde_json::from_str(&json)?;
            select_section(&mut monitoring, section, device);
            points.push(MonitoringHistoryPoint {
                collected_at: parse_time(collected_at)?,
                received_at: parse_time(received_at)?,
                monitoring,
            });
        }
        // Event histories use deduplicated result tables so a time-bucketed host snapshot cannot hide a probe or service check.
        if section == "probes" || section == "services" {
            points.clear();
            let (table, time_column, id_column) = if section == "probes" {
                ("probe_samples", "scheduled_at", "target_id")
            } else {
                ("service_samples", "checked_at", "subject_id")
            };
            let query = format!(
                "SELECT {time_column}, received_at, result_json FROM {table}
                WHERE node_id = ?1 AND received_at >= ?2 AND (?3 IS NULL OR {id_column} = ?3)
                ORDER BY received_at DESC LIMIT ?4"
            );
            let mut statement = connection.prepare(&query)?;
            let rows = statement.query_map(
                params![node_id.to_string(), cutoff, device, MAX_HISTORY_POINTS],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )?;
            for row in rows {
                let (collected_at, received_at, json) = row?;
                let mut monitoring = MonitoringData::default();
                if section == "probes" {
                    monitoring.probes.push(serde_json::from_str(&json)?);
                } else {
                    monitoring.services.push(serde_json::from_str(&json)?);
                }
                points.push(MonitoringHistoryPoint {
                    collected_at: parse_time(collected_at)?,
                    received_at: parse_time(received_at)?,
                    monitoring,
                });
            }
            points.reverse();
        }
        Ok(Some(points))
    }

    pub fn probe_statistics(
        &self,
        node_id: Uuid,
        minutes: u64,
    ) -> Result<Option<Vec<ProbeStatistics>>> {
        let connection = self.lock()?;
        if !node_exists(&connection, node_id)? {
            return Ok(None);
        }
        let end = Utc::now();
        let start = end - chrono::Duration::minutes(minutes as i64);
        let mut statement = connection.prepare("SELECT effective_at, config_json FROM monitoring_configs WHERE node_id = ?1 ORDER BY revision")?;
        let configs = statement
            .query_map([node_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .map(|row| {
                let (at, json) = row?;
                Ok((
                    parse_time(at)?,
                    serde_json::from_str::<NodeMonitoringConfig>(&json)?,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut statement = connection.prepare("SELECT result_json FROM probe_samples WHERE node_id = ?1 AND scheduled_at >= ?2 AND scheduled_at <= ?3 ORDER BY scheduled_at")?;
        let samples = statement
            .query_map(
                params![node_id.to_string(), timestamp(start), timestamp(end)],
                |row| row.get::<_, String>(0),
            )?
            .map(|row| Ok(serde_json::from_str::<ProbeResult>(&row?)?));
        Ok(Some(calculate_statistics(&configs, samples, start, end)?))
    }
}

fn node_exists(connection: &Connection, node_id: Uuid) -> Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM nodes WHERE id = ?1",
            [node_id.to_string()],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

pub(crate) fn insert_results(
    transaction: &Transaction<'_>,
    node_id: Uuid,
    report: &MetricReport,
    received_at: DateTime<Utc>,
) -> Result<()> {
    let Some(data) = &report.monitoring else {
        return Ok(());
    };
    for probe in &data.probes {
        transaction.execute("INSERT OR IGNORE INTO probe_samples(node_id, sample_id, target_id, config_revision, scheduled_at, received_at, result_json) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![node_id.to_string(), probe.sample_id.to_string(), probe.target_id.to_string(), i64::try_from(probe.config_revision)?, timestamp(probe.scheduled_at), timestamp(received_at), serde_json::to_string(probe)?])?;
    }
    for service in &data.services {
        transaction.execute("INSERT OR IGNORE INTO service_samples(node_id, subject_id, config_revision, checked_at, received_at, result_json) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![node_id.to_string(), service.id.to_string(), i64::try_from(service.config_revision)?, timestamp(service.checked_at), timestamp(received_at), serde_json::to_string(service)?])?;
    }
    Ok(())
}

fn select_section(data: &mut MonitoringData, section: &str, device: Option<&str>) {
    if let Some(device) = device {
        data.cpu_cores.retain(|core| core.id == device);
        data.disk_io.retain(|disk| disk.id == device);
        data.inodes
            .retain(|inode| inode.id == device || inode.mount_point == device);
        data.network_health
            .retain(|interface| interface.id == device || interface.name == device);
        data.services
            .retain(|service| service.id.to_string() == device);
        data.probes
            .retain(|probe| probe.target_id.to_string() == device);
    }
    if section == "all" {
        return;
    }
    if section != "cpu" {
        data.cpu_cores.clear();
        data.cpu_times = Default::default();
    }
    if section != "memory" {
        data.memory = Default::default();
    }
    if section != "disk" {
        data.disk_io.clear();
        data.inodes.clear();
    }
    if section != "network" {
        data.network_health.clear();
    }
    if section != "tcp" {
        data.tcp = Default::default();
    }
    if section != "agent" {
        data.agent = Default::default();
    }
    if section != "services" {
        data.services.clear();
    }
    if section != "probes" {
        data.probes.clear();
    }
}

fn calculate_statistics(
    configs: &[(DateTime<Utc>, NodeMonitoringConfig)],
    samples: impl Iterator<Item = Result<ProbeResult>>,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<ProbeStatistics>> {
    let mut results: BTreeMap<Uuid, (ProbeStatistics, Vec<f64>, u64)> = BTreeMap::new();
    let mut periods = BTreeMap::new();
    let current_targets = configs
        .last()
        .map(|(_, config)| {
            config
                .probes
                .iter()
                .map(|target| (target.id, target))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    if current_targets.is_empty() {
        return Ok(Vec::new());
    }
    for (index, (effective_at, config)) in configs.iter().enumerate() {
        let period_start = (*effective_at).max(start);
        let period_end = configs
            .get(index + 1)
            .map(|(at, _)| *at)
            .unwrap_or(end)
            .min(end);
        if period_start >= period_end {
            continue;
        }
        for target in &config.probes {
            let Some(current) = current_targets.get(&target.id) else {
                continue;
            };
            if !same_probe_definition(target, current) {
                continue;
            }
            let (stats, _, _) = results.entry(target.id).or_insert_with(|| {
                (
                    ProbeStatistics {
                        target_id: target.id,
                        name: current.name.clone(),
                        kind: current.kind,
                        successful: 0,
                        failed: 0,
                        unknown: 0,
                        expected: 0,
                        success_rate_percent: None,
                        coverage_percent: None,
                        p50_ms: None,
                        p95_ms: None,
                        p99_ms: None,
                        latency_samples: 0,
                        observed_seconds: 0.0,
                        available_seconds: 0.0,
                        unknown_seconds: 0.0,
                    },
                    Vec::new(),
                    0,
                )
            });
            if !target.enabled {
                continue;
            }
            let interval_ms = (target.interval_secs * 1000) as i64;
            let first_slot = period_start
                .signed_duration_since(*effective_at)
                .num_milliseconds()
                .max(0) as u64
                / interval_ms as u64;
            let last_slot = (period_end
                .signed_duration_since(*effective_at)
                .num_milliseconds()
                .max(0) as u64)
                .div_ceil(interval_ms as u64);
            stats.expected += last_slot.saturating_sub(first_slot);
            let active_seconds = period_end
                .signed_duration_since(period_start)
                .num_milliseconds() as f64
                / 1000.0;
            stats.unknown_seconds += active_seconds;
            periods.insert(
                (config.revision, target.id),
                (
                    period_start,
                    period_end,
                    target.interval_secs,
                    target.kind,
                    *effective_at,
                    None,
                    None,
                ),
            );
        }
    }
    // Stream serialized samples; only latency scalars are retained for exact quantiles.
    for sample in samples {
        let sample = sample?;
        let Some((period_start, period_end, interval, kind, effective_at, last_slot, slot_health)) =
            periods.get_mut(&(sample.config_revision, sample.target_id))
        else {
            continue;
        };
        if sample.scheduled_at < *period_start
            || sample.scheduled_at >= *period_end
            || sample.kind != *kind
        {
            continue;
        }
        let (stats, latencies, covered_slots) = results
            .get_mut(&sample.target_id)
            .expect("period target exists");
        let slot = sample
            .scheduled_at
            .signed_duration_since(*effective_at)
            .num_milliseconds()
            / (*interval * 1000) as i64;
        if *last_slot != Some(slot) {
            *last_slot = Some(slot);
            *slot_health = None;
        }
        let slot_start = *effective_at + chrono::Duration::seconds(slot * *interval as i64);
        let slot_end = slot_start + chrono::Duration::seconds(*interval as i64);
        let duration = (slot_end.min(*period_end) - slot_start.max(*period_start))
            .num_milliseconds()
            .max(0) as f64
            / 1000.0;
        let healthy = match sample.status {
            ProbeStatus::Success => {
                stats.successful += 1;
                if let Some(latency) = sample.latency_ms {
                    latencies.push(latency);
                }
                Some(true)
            }
            ProbeStatus::Failure | ProbeStatus::Timeout => {
                stats.failed += 1;
                Some(false)
            }
            _ => {
                stats.unknown += 1;
                None
            }
        };
        if let Some(healthy) = healthy {
            if slot_health.is_none() {
                stats.observed_seconds += duration;
                stats.unknown_seconds = (stats.unknown_seconds - duration).max(0.0);
                *covered_slots += 1;
            } else if *slot_health == Some(true) {
                stats.available_seconds -= duration;
            }
            if healthy {
                stats.available_seconds += duration;
            }
            *slot_health = Some(healthy);
        }
    }
    Ok(results
        .into_values()
        .map(|(mut stats, mut latencies, covered_slots)| {
            let attempts = stats.successful + stats.failed;
            stats.success_rate_percent =
                (attempts > 0).then(|| stats.successful as f64 * 100.0 / attempts as f64);
            stats.coverage_percent = (stats.expected > 0)
                .then(|| (covered_slots as f64 * 100.0 / stats.expected as f64).min(100.0));
            stats.latency_samples = latencies.len() as u64;
            latencies.sort_by(f64::total_cmp);
            stats.p50_ms = percentile(&latencies, 50);
            stats.p95_ms = percentile(&latencies, 95);
            stats.p99_ms = percentile(&latencies, 99);
            stats
        })
        .collect())
}

fn same_probe_definition(
    left: &pinglake_protocol::ProbeTarget,
    right: &pinglake_protocol::ProbeTarget,
) -> bool {
    left.kind == right.kind
        && left.target == right.target
        && left.port == right.port
        && left.expected_status == right.expected_status
        && left.response_contains == right.response_contains
}

fn percentile(values: &[f64], percent: usize) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values[(values.len() * percent).div_ceil(100) - 1])
    }
}

fn invalid(message: &str) -> AppError {
    AppError::bad_request(message)
}

fn bounded(value: &str, maximum: usize, allow_empty: bool) -> Result<(), AppError> {
    if value.len() > maximum
        || (!allow_empty && value.trim().is_empty())
        || value.chars().any(char::is_control)
    {
        Err(invalid(
            "monitoring string is empty, too long, or contains control characters",
        ))
    } else {
        Ok(())
    }
}

fn bounded_error(value: &str) -> Result<(), AppError> {
    if value.len() > 1024
        || value
            .chars()
            .any(|character| character.is_control() && !"\r\n\t".contains(character))
    {
        Err(invalid(
            "monitoring error exceeds 1024 bytes or contains invalid characters",
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn validate_config(config: &NodeMonitoringConfig) -> Result<(), AppError> {
    if config.services.len() > 32 || config.process_checks.len() > 32 || config.probes.len() > 32 {
        return Err(invalid("at most 32 services and 32 probes are allowed"));
    }
    if let Some(value) = &config.browser_latency_url {
        bounded(value, 2048, false)?;
        let url = reqwest::Url::parse(value)
            .map_err(|_| invalid("browser latency URL must be an absolute HTTPS URL"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(invalid(
                "browser latency URL requires HTTPS, a host, and no credentials or fragment",
            ));
        }
    }
    let mut ids = HashSet::new();
    for service in &config.services {
        if service.id.is_nil() || !ids.insert(service.id) {
            return Err(invalid("service IDs must be non-nil and unique"));
        }
        bounded(&service.name, 256, false)?;
        if service.name.starts_with('-')
            || service
                .name
                .chars()
                .any(|value| !value.is_ascii_alphanumeric() && !"._@- ".contains(value))
        {
            return Err(invalid("service name contains unsupported characters"));
        }
        if !["running", "stopped"].contains(&service.expected_state.as_str()) {
            return Err(invalid("service expected_state must be running or stopped"));
        }
    }
    for process in &config.process_checks {
        if process.id.is_nil() || !ids.insert(process.id) {
            return Err(invalid("process IDs must be non-nil and unique"));
        }
        bounded(&process.name, 256, false)?;
        bounded(&process.process_name, 256, false)?;
        if process
            .process_name
            .chars()
            .any(|c| c == '/' || c == '\\' || c.is_control())
            || !["running", "stopped"].contains(&process.expected_state.as_str())
        {
            return Err(invalid("invalid process configuration"));
        }
    }
    ids.clear();
    for probe in &config.probes {
        if probe.id.is_nil() || !ids.insert(probe.id) {
            return Err(invalid("probe IDs must be non-nil and unique"));
        }
        bounded(&probe.name, 128, false)?;
        bounded(&probe.target, 2048, false)?;
        if !(10..=86_400).contains(&probe.interval_secs)
            || probe.timeout_ms == 0
            || probe.timeout_ms > 10_000
            || probe.timeout_ms >= probe.interval_secs * 1000
        {
            return Err(invalid(
                "probe interval must be 10-86400 seconds and timeout 1-10000 ms below its interval",
            ));
        }
        if probe.port == Some(0) {
            return Err(invalid("probe port must be 1-65535"));
        }
        if probe
            .expected_status
            .is_some_and(|status| !(100..=599).contains(&status))
        {
            return Err(invalid("HTTP expected status must be 100-599"));
        }
        if let Some(value) = &probe.response_contains {
            bounded(value, 1024, true)?;
        }
        match probe.kind {
            ProbeKind::Http => {
                let url = reqwest::Url::parse(&probe.target)
                    .map_err(|_| invalid("HTTP probe needs an absolute URL"))?;
                if !["http", "https"].contains(&url.scheme())
                    || url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                    || url.fragment().is_some()
                {
                    return Err(invalid(
                        "HTTP target must use HTTP(S) without embedded credentials",
                    ));
                }
            }
            ProbeKind::Tcp | ProbeKind::Icmp | ProbeKind::Dns => {
                if probe.target.len() > 253
                    || probe.target.starts_with('-')
                    || probe
                        .target
                        .chars()
                        .any(|value| !value.is_ascii_alphanumeric() && !".:-".contains(value))
                {
                    return Err(invalid("TCP/ICMP target must be a hostname or IP address"));
                }
                if probe.kind == ProbeKind::Tcp && probe.port.is_none() {
                    return Err(invalid("TCP probe requires a port"));
                }
                if probe.kind == ProbeKind::Dns && probe.port.is_some() {
                    return Err(invalid("DNS probe does not accept a port"));
                }
            }
            // The protocol reserves these kinds for Agents that implement the
            // corresponding extension. Older Hubs persist and relay them;
            // execution support is intentionally outside this module.
            ProbeKind::Dns | ProbeKind::Process | ProbeKind::LocalPort => {
                bounded(&probe.target, 253, true)?;
                if probe.kind == ProbeKind::LocalPort && probe.port.is_none() {
                    return Err(invalid("local port probe requires a port"));
                }
            }
        }
    }
    Ok(())
}

fn nonnegative(value: Option<f64>) -> Result<(), AppError> {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
        Err(invalid("monitoring number must be finite and nonnegative"))
    } else {
        Ok(())
    }
}

fn percent(value: Option<f64>) -> Result<(), AppError> {
    nonnegative(value)?;
    if value.is_some_and(|value| value > 100.0) {
        Err(invalid("monitoring percentage exceeds 100"))
    } else {
        Ok(())
    }
}

fn counter(value: Option<&str>) -> Result<(), AppError> {
    if let Some(value) = value
        && (value.is_empty()
            || value.len() > 20
            || !value.bytes().all(|value| value.is_ascii_digit())
            || value.parse::<u64>().is_err())
    {
        return Err(invalid("monitoring counter must be a decimal u64 string"));
    }
    Ok(())
}

pub(crate) fn validate_data(data: &MonitoringData) -> Result<(), AppError> {
    if data.schema_version > 1
        || data.report_interval_secs > 86_400
        || data.cpu_cores.len() > 1024
        || data.disk_io.len() > 128
        || data.inodes.len() > 128
        || data.network_health.len() > 128
        || data.services.len() > 32
        || data.probes.len() > 128
        || data.capabilities.len() > 32
        || data.tcp.states.len() > 32
    {
        return Err(invalid(
            "monitoring report exceeds supported version or collection limits",
        ));
    }
    for (name, capability) in &data.capabilities {
        bounded(name, 64, false)?;
        bounded(&capability.source, 128, true)?;
        if let Some(error) = &capability.error {
            bounded_error(error)?;
        }
    }
    let mut ids = HashSet::new();
    for core in &data.cpu_cores {
        bounded(&core.id, 128, false)?;
        if !ids.insert(&core.id) {
            return Err(invalid("CPU IDs must be unique"));
        }
        percent(Some(f64::from(core.usage_percent)))?;
    }
    for value in [
        data.cpu_times.user_percent,
        data.cpu_times.system_percent,
        data.cpu_times.iowait_percent,
        data.cpu_times.steal_percent,
        data.agent.success_rate_percent,
    ] {
        percent(value)?;
    }
    for value in [
        data.memory.swap_in_bytes_per_sec,
        data.memory.swap_out_bytes_per_sec,
        Some(data.agent.collection_duration_ms),
        data.agent.send_duration_ms,
        Some(data.agent.sample_age_ms),
    ] {
        nonnegative(value)?;
    }
    counter(data.memory.swap_in_bytes.as_deref())?;
    counter(data.memory.swap_out_bytes.as_deref())?;
    if data.agent.upload_successes > data.agent.upload_attempts
        || data.agent.upload_failures > data.agent.upload_attempts
        || data
            .agent
            .upload_successes
            .checked_add(data.agent.upload_failures)
            .is_none_or(|total| total > data.agent.upload_attempts)
    {
        return Err(invalid("Agent upload counters are inconsistent"));
    }
    for error in [&data.agent.last_error, &data.agent.config_error]
        .into_iter()
        .flatten()
    {
        bounded_error(error)?;
    }
    ids.clear();
    for disk in &data.disk_io {
        bounded(&disk.id, 256, false)?;
        bounded(&disk.name, 256, true)?;
        if !ids.insert(&disk.id) {
            return Err(invalid("disk IDs must be unique"));
        }
        for value in [
            &disk.read_bytes,
            &disk.written_bytes,
            &disk.reads,
            &disk.writes,
        ] {
            counter(value.as_deref())?;
        }
        for value in [
            disk.read_bytes_per_sec,
            disk.write_bytes_per_sec,
            disk.read_iops,
            disk.write_iops,
            disk.read_latency_ms,
            disk.write_latency_ms,
            disk.queue_depth,
        ] {
            nonnegative(value)?;
        }
        percent(disk.utilization_percent)?;
    }
    ids.clear();
    for inode in &data.inodes {
        bounded(&inode.id, 512, false)?;
        bounded(&inode.mount_point, 512, true)?;
        if !ids.insert(&inode.id) {
            return Err(invalid("inode IDs must be unique"));
        }
        for value in [&inode.total, &inode.used, &inode.free] {
            counter(value.as_deref())?;
        }
        percent(inode.used_percent)?;
        if let (Some(total), Some(used)) = (&inode.total, &inode.used)
            && used.parse::<u64>().unwrap() > total.parse::<u64>().unwrap()
        {
            return Err(invalid("inode used exceeds total"));
        }
    }
    ids.clear();
    for interface in &data.network_health {
        bounded(&interface.id, 256, false)?;
        bounded(&interface.name, 256, true)?;
        if !ids.insert(&interface.id) {
            return Err(invalid("network IDs must be unique"));
        }
        for value in [
            &interface.received_bytes,
            &interface.transmitted_bytes,
            &interface.received_packets,
            &interface.transmitted_packets,
            &interface.receive_errors,
            &interface.transmit_errors,
        ] {
            if interface.status == MetricStatus::Ok || !value.is_empty() {
                counter(Some(value))?;
            }
        }
        counter(interface.receive_drops.as_deref())?;
        counter(interface.transmit_drops.as_deref())?;
        for value in [
            interface.receive_errors_per_sec,
            interface.transmit_errors_per_sec,
            interface.receive_drops_per_sec,
            interface.transmit_drops_per_sec,
        ] {
            nonnegative(value)?;
        }
    }
    bounded(&data.tcp.scope, 128, true)?;
    for state in data.tcp.states.keys() {
        bounded(state, 64, false)?;
    }
    let now = Utc::now();
    let earliest = now - chrono::Duration::days(7);
    let latest = now + chrono::Duration::minutes(5);
    for service in &data.services {
        if service.id.is_nil() || service.config_revision > i64::MAX as u64 {
            return Err(invalid("invalid service identity or revision"));
        }
        bounded(&service.name, 256, false)?;
        bounded(&service.state, 128, true)?;
        if let Some(error) = &service.error {
            bounded_error(error)?;
        }
        if service.checked_at < earliest || service.checked_at > latest {
            return Err(invalid("service check time outside accepted window"));
        }
        if service.status != MetricStatus::Ok && service.healthy.is_some() {
            return Err(invalid(
                "unavailable service cannot report a health verdict",
            ));
        }
    }
    let mut sample_ids = HashSet::new();
    for probe in &data.probes {
        if probe.sample_id.is_nil()
            || probe.target_id.is_nil()
            || !sample_ids.insert(probe.sample_id)
            || probe.config_revision > i64::MAX as u64
        {
            return Err(invalid("invalid or repeated probe identity"));
        }
        nonnegative(probe.latency_ms)?;
        if probe.scheduled_at < earliest
            || probe.scheduled_at > latest
            || probe.completed_at < probe.scheduled_at
            || probe.completed_at > latest
        {
            return Err(invalid("probe timestamps outside accepted window"));
        }
        if probe.status == ProbeStatus::Success && probe.latency_ms.is_none() {
            return Err(invalid("successful probe needs a latency"));
        }
        if probe
            .http_status
            .is_some_and(|status| !(100..=599).contains(&status))
        {
            return Err(invalid("invalid HTTP result status"));
        }
        if let Some(error) = &probe.error {
            bounded_error(error)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pinglake_protocol::{CpuCore, ProbeTarget};

    fn target(id: Uuid, interval_secs: u64) -> ProbeTarget {
        ProbeTarget {
            id,
            name: "Example".into(),
            kind: ProbeKind::Http,
            target: "https://example.com/".into(),
            port: None,
            enabled: true,
            interval_secs,
            timeout_ms: 1000,
            expected_status: Some(200),
            response_contains: None,
        }
    }

    fn sample(
        id: Uuid,
        revision: u64,
        at: DateTime<Utc>,
        status: ProbeStatus,
        latency: Option<f64>,
    ) -> ProbeResult {
        ProbeResult {
            sample_id: Uuid::new_v4(),
            target_id: id,
            config_revision: revision,
            kind: ProbeKind::Http,
            scheduled_at: at,
            completed_at: at,
            status,
            latency_ms: latency,
            http_status: None,
            error: None,
        }
    }

    #[test]
    fn statistics_separate_actual_attempts_from_versioned_slot_coverage() {
        let start = Utc::now() - chrono::Duration::minutes(10);
        let id = Uuid::new_v4();
        let first = NodeMonitoringConfig {
            revision: 1,
            probes: vec![target(id, 30)],
            ..Default::default()
        };
        let mut paused = first.clone();
        paused.revision = 2;
        paused.probes[0].enabled = false;
        let resumed = NodeMonitoringConfig {
            revision: 3,
            probes: vec![target(id, 10)],
            ..Default::default()
        };
        let at = |seconds| start + chrono::Duration::seconds(seconds);
        let configs = vec![(at(0), first), (at(60), paused), (at(90), resumed)];
        let samples = vec![
            sample(id, 1, at(0), ProbeStatus::Success, Some(1.0)),
            sample(id, 1, at(30), ProbeStatus::Timeout, None),
            sample(id, 1, at(75), ProbeStatus::Success, Some(999.0)),
            sample(id, 3, at(90), ProbeStatus::PermissionDenied, None),
            sample(id, 3, at(100), ProbeStatus::Success, Some(10.0)),
            sample(id, 3, at(101), ProbeStatus::Success, Some(20.0)),
        ];
        let stats = calculate_statistics(&configs, samples.into_iter().map(Ok), at(0), at(120))
            .unwrap()
            .remove(0);
        assert_eq!(
            (
                stats.successful,
                stats.failed,
                stats.unknown,
                stats.expected
            ),
            (3, 1, 1, 5)
        );
        assert_eq!(stats.success_rate_percent, Some(75.0));
        assert_eq!(stats.coverage_percent, Some(60.0));
        assert_eq!(stats.latency_samples, 3);
        assert_eq!(
            (stats.p50_ms, stats.p95_ms, stats.p99_ms),
            (Some(10.0), Some(20.0), Some(20.0))
        );
        assert_eq!(
            (
                stats.observed_seconds,
                stats.available_seconds,
                stats.unknown_seconds
            ),
            (70.0, 40.0, 20.0)
        );
    }

    #[test]
    fn missing_or_unsupported_checks_do_not_fabricate_availability_or_latency() {
        let start = Utc::now();
        let id = Uuid::new_v4();
        let configs = vec![(
            start,
            NodeMonitoringConfig {
                revision: 1,
                probes: vec![target(id, 30)],
                ..Default::default()
            },
        )];
        let samples = vec![sample(id, 1, start, ProbeStatus::Unsupported, None)];
        let stats = calculate_statistics(
            &configs,
            samples.into_iter().map(Ok),
            start,
            start + chrono::Duration::seconds(60),
        )
        .unwrap()
        .remove(0);
        assert_eq!(stats.expected, 2);
        assert_eq!(stats.success_rate_percent, None);
        assert_eq!(stats.coverage_percent, Some(0.0));
        assert_eq!(stats.p99_ms, None);
        assert_eq!(stats.latency_samples, 0);
        assert_eq!(stats.unknown_seconds, 60.0);
        assert_eq!(percentile(&[7.0], 99), Some(7.0));
    }

    #[test]
    fn editing_probe_definition_excludes_old_latency_but_interval_and_browser_updates_merge() {
        let start = Utc::now() - chrono::Duration::minutes(10);
        let at = |seconds| start + chrono::Duration::seconds(seconds);
        for changed_field in 0..5 {
            let id = Uuid::new_v4();
            let current = target(id, 30);
            let mut previous = current.clone();
            match changed_field {
                0 => {
                    previous.kind = ProbeKind::Tcp;
                    previous.target = "example.com".into();
                    previous.port = Some(443);
                }
                1 => previous.target = "https://example.com/old-path".into(),
                2 => previous.port = Some(8443),
                3 => previous.expected_status = Some(404),
                _ => previous.response_contains = Some("old matcher".into()),
            }
            let old_kind = previous.kind;
            let first = NodeMonitoringConfig {
                revision: 1,
                probes: vec![previous],
                ..Default::default()
            };
            let second = NodeMonitoringConfig {
                revision: 2,
                probes: vec![current.clone()],
                ..Default::default()
            };
            let mut third = NodeMonitoringConfig {
                revision: 3,
                browser_latency_url: Some("https://browser.example.com/ping".into()),
                probes: vec![current],
                ..Default::default()
            };
            third.probes[0].interval_secs = 10;
            third.probes[0].name = "Current name".into();
            let configs = vec![(at(0), first), (at(60), second), (at(90), third)];
            let mut obsolete = sample(id, 1, at(0), ProbeStatus::Success, Some(999.0));
            obsolete.kind = old_kind;
            let samples = vec![
                obsolete,
                sample(id, 2, at(60), ProbeStatus::Success, Some(5.0)),
                sample(id, 3, at(90), ProbeStatus::Success, Some(10.0)),
                sample(id, 3, at(100), ProbeStatus::Timeout, None),
            ];
            let statistics =
                calculate_statistics(&configs, samples.into_iter().map(Ok), at(0), at(120))
                    .unwrap();
            assert_eq!(statistics.len(), 1);
            let stats = &statistics[0];
            assert_eq!(stats.kind, ProbeKind::Http);
            assert_eq!(stats.name, "Current name");
            assert_eq!((stats.successful, stats.failed, stats.expected), (2, 1, 4));
            assert_eq!(stats.p95_ms, Some(10.0));
            assert_eq!(stats.latency_samples, 2);
            assert_eq!(stats.coverage_percent, Some(75.0));
            assert_eq!(
                (
                    stats.observed_seconds,
                    stats.available_seconds,
                    stats.unknown_seconds
                ),
                (50.0, 40.0, 10.0)
            );
        }
    }

    #[test]
    fn removed_targets_do_not_appear_in_current_statistics() {
        let start = Utc::now();
        let id = Uuid::new_v4();
        let configs = vec![
            (
                start,
                NodeMonitoringConfig {
                    revision: 1,
                    probes: vec![target(id, 30)],
                    ..Default::default()
                },
            ),
            (
                start + chrono::Duration::seconds(30),
                NodeMonitoringConfig {
                    revision: 2,
                    ..Default::default()
                },
            ),
        ];
        let samples = vec![sample(id, 1, start, ProbeStatus::Success, Some(999.0))];
        assert!(
            calculate_statistics(
                &configs,
                samples.into_iter().map(Ok),
                start,
                start + chrono::Duration::seconds(60)
            )
            .unwrap()
            .is_empty()
        );
    }

    #[test]
    fn config_rejects_credentials_fragments_duplicate_ids_and_unsafe_intervals() {
        let id = Uuid::new_v4();
        let mut config = NodeMonitoringConfig {
            probes: vec![target(id, 30)],
            ..Default::default()
        };
        assert!(validate_config(&config).is_ok());
        for url in [
            "http://example.com/ping",
            "https://user:secret@example.com/ping",
            "https://example.com/ping#fragment",
        ] {
            config.browser_latency_url = Some(url.into());
            assert!(validate_config(&config).is_err());
        }
        config.browser_latency_url = Some("https://node.example.com/ping".into());
        config.probes.push(target(id, 30));
        assert!(validate_config(&config).is_err());
        config.probes.pop();
        config.probes[0].interval_secs = 10;
        config.probes[0].timeout_ms = 10_000;
        assert!(validate_config(&config).is_err());
        config.probes[0].timeout_ms = 9999;
        assert!(validate_config(&config).is_ok());
    }

    #[test]
    fn monitoring_validation_preserves_u64_counters_and_rejects_invalid_numbers() {
        let mut data = MonitoringData::default();
        data.memory.swap_in_bytes = Some(u64::MAX.to_string());
        assert!(validate_data(&data).is_ok());
        data.memory.swap_in_bytes = Some("18446744073709551616".into());
        assert!(validate_data(&data).is_err());
        data.memory.swap_in_bytes = None;
        data.cpu_cores.push(CpuCore {
            id: "0".into(),
            usage_percent: f32::NAN,
            frequency_mhz: None,
        });
        assert!(validate_data(&data).is_err());
        data.cpu_cores[0].usage_percent = 10.0;
        data.cpu_cores.push(data.cpu_cores[0].clone());
        assert!(validate_data(&data).is_err());
    }
}
