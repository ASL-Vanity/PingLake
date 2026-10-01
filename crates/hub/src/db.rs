use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use pinglake_protocol::{
    AlertKind, AlertRecord, AlertSettings, EnrollRequest, HistoryPoint, HostGroup, MetricReport,
    NodeSnapshot,
};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use uuid::Uuid;

const METRIC_RETENTION_DAYS: i64 = 7;
const MAX_HISTORY_POINTS: usize = 1_440;

pub struct Database {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnrollResult {
    Created,
    Updated,
    SecretMismatch,
    EnrollmentTokenRequired,
}

pub struct MetricResult {
    pub snapshot: NodeSnapshot,
    pub alerts: Vec<AlertRecord>,
}

#[derive(Debug)]
struct NodeRow {
    id: Uuid,
    hostname: String,
    display_name: String,
    os: String,
    os_version: String,
    kernel_version: String,
    architecture: String,
    agent_version: String,
    group_id: Option<Uuid>,
    group_name: Option<String>,
    browser_latency_url: Option<String>,
    enrolled_at: DateTime<Utc>,
    last_seen_at: Option<DateTime<Utc>>,
}

impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create database directory {}", parent.display())
            })?;
        }

        let connection = Connection::open(path)
            .with_context(|| format!("failed to open database {}", path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&connection)?;

        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn enroll(
        &self,
        request: &EnrollRequest,
        secret_hash: &str,
        enrollment_token_valid: bool,
    ) -> Result<EnrollResult> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let existing_hash = transaction
            .query_row(
                "SELECT secret_hash FROM nodes WHERE id = ?1",
                [request.agent_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;

        let result = match existing_hash {
            Some(existing_hash) if !constant_time_equal(&existing_hash, secret_hash) => {
                EnrollResult::SecretMismatch
            }
            Some(_) => {
                transaction.execute(
                    "UPDATE nodes SET hostname = ?2, display_name = CASE WHEN display_name_overridden = 1 THEN display_name ELSE ?3 END, os = ?4, \
                     os_version = ?5, kernel_version = ?6, architecture = ?7, agent_version = ?8 \
                     WHERE id = ?1",
                    params![
                        request.agent_id.to_string(),
                        request.hostname,
                        request.display_name,
                        request.os,
                        request.os_version,
                        request.kernel_version,
                        request.architecture,
                        request.agent_version,
                    ],
                )?;
                EnrollResult::Updated
            }
            None if !enrollment_token_valid => EnrollResult::EnrollmentTokenRequired,
            None => {
                transaction.execute(
                    "INSERT INTO nodes (
                        id, secret_hash, hostname, display_name, os, os_version,
                        kernel_version, architecture, agent_version, enrolled_at
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        request.agent_id.to_string(),
                        secret_hash,
                        request.hostname,
                        request.display_name,
                        request.os,
                        request.os_version,
                        request.kernel_version,
                        request.architecture,
                        request.agent_version,
                        timestamp(Utc::now()),
                    ],
                )?;
                EnrollResult::Created
            }
        };

        transaction.commit()?;
        Ok(result)
    }

    pub fn record_metric(
        &self,
        node_id: Uuid,
        secret_hash: &str,
        report: &MetricReport,
    ) -> Result<Option<MetricResult>> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let authenticated = transaction
            .query_row(
                "SELECT secret_hash FROM nodes WHERE id = ?1",
                [node_id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_some_and(|stored| constant_time_equal(&stored, secret_hash));
        if !authenticated {
            return Ok(None);
        }

        let now = Utc::now();
        if let Some(data) = &report.monitoring
            && !data.session_id.is_nil()
        {
            let duplicate = transaction.query_row(
                    "SELECT 1 FROM metrics WHERE node_id = ?1 AND monitoring_session = ?2 AND monitoring_sequence = ?3",
                    params![node_id.to_string(), data.session_id.to_string(), data.sample_sequence.to_string()],
                    |_| Ok(()),
                ).optional()?.is_some();
            if duplicate {
                transaction.execute(
                    "UPDATE nodes SET last_seen_at = ?2 WHERE id = ?1",
                    params![node_id.to_string(), timestamp(now)],
                )?;
                let node = get_node_row(&transaction, node_id)?
                    .ok_or_else(|| anyhow!("node disappeared"))?;
                let latest = latest_metric(&transaction, node_id)?;
                let settings = get_settings_from(&transaction)?;
                transaction.commit()?;
                return Ok(Some(MetricResult {
                    snapshot: node_snapshot(node, latest, &settings, now),
                    alerts: Vec::new(),
                }));
            }
        }
        insert_metric(&transaction, node_id, report, now)?;
        crate::monitoring::insert_results(&transaction, node_id, report, now)?;
        transaction.execute(
            "UPDATE nodes SET last_seen_at = ?2 WHERE id = ?1",
            params![node_id.to_string(), timestamp(now)],
        )?;

        let node =
            get_node_row(&transaction, node_id)?.ok_or_else(|| anyhow!("node disappeared"))?;
        let settings = get_settings_from(&transaction)?;
        let mut alerts = Vec::new();

        if let Some(resolved) = resolve_alert(&transaction, node_id, &AlertKind::Offline, now)? {
            alerts.push(resolved);
        }

        let memory_percent = percent(report.memory_used_bytes, report.memory_total_bytes);
        let disk_percent = percent(report.disk_used_bytes, report.disk_total_bytes);
        let checks = [
            settings.cpu_enabled.then_some(ThresholdCheck {
                kind: AlertKind::Cpu,
                value: Some(f64::from(report.cpu_percent)),
                threshold: settings.cpu_percent,
                violated: f64::from(report.cpu_percent) >= settings.cpu_percent,
                label: "CPU usage",
                unit: "%",
            }),
            settings.memory_enabled.then_some(ThresholdCheck {
                kind: AlertKind::Memory,
                value: memory_percent,
                threshold: settings.memory_percent,
                violated: memory_percent.is_some_and(|value| value >= settings.memory_percent),
                label: "Memory usage",
                unit: "%",
            }),
            settings.disk_enabled.then_some(ThresholdCheck {
                kind: AlertKind::Disk,
                value: disk_percent,
                threshold: settings.disk_percent,
                violated: disk_percent.is_some_and(|value| value >= settings.disk_percent),
                label: "Disk usage",
                unit: "%",
            }),
            settings.temperature_enabled.then_some(ThresholdCheck {
                kind: AlertKind::Temperature,
                value: report.temperature_celsius.map(f64::from),
                threshold: settings.temperature_celsius,
                violated: report
                    .temperature_celsius
                    .is_some_and(|value| f64::from(value) >= settings.temperature_celsius),
                label: "Temperature",
                unit: " C",
            }),
        ];

        for check in checks {
            let Some(check) = check else { continue };
            if let Some(alert) = apply_threshold_check(
                &transaction,
                &node,
                check,
                settings.sustained_for_seconds,
                now,
            )? {
                alerts.push(alert);
            }
        }
        alerts.extend(apply_monitoring_alerts(
            &transaction,
            &node,
            report,
            &settings,
            now,
        )?);

        transaction.commit()?;
        Ok(Some(MetricResult {
            snapshot: NodeSnapshot {
                id: node.id,
                hostname: node.hostname,
                display_name: node.display_name,
                os: node.os,
                os_version: node.os_version,
                kernel_version: node.kernel_version,
                architecture: node.architecture,
                agent_version: node.agent_version,
                group_id: node.group_id,
                group_name: node.group_name,
                enrolled_at: node.enrolled_at,
                last_seen_at: Some(now),
                online: true,
                latest: Some(report.clone()),
                browser_latency_url: node.browser_latency_url,
            },
            alerts,
        }))
    }

    pub fn nodes(&self) -> Result<Vec<NodeSnapshot>> {
        let connection = self.lock()?;
        let settings = get_settings_from(&connection)?;
        let now = Utc::now();
        let mut statement = connection.prepare(
            "SELECT nodes.id, hostname, display_name, os, os_version, kernel_version,
                    architecture, agent_version, enrolled_at, last_seen_at, nodes.group_id, groups.name, nodes.browser_latency_url
             FROM nodes LEFT JOIN groups ON groups.id = nodes.group_id
             ORDER BY groups.name COLLATE NOCASE, display_name COLLATE NOCASE, hostname COLLATE NOCASE",
        )?;
        let rows = statement.query_map([], map_node_row)?;
        let mut snapshots = Vec::new();
        for row in rows {
            let node = row?;
            let latest = latest_metric(&connection, node.id)?;
            snapshots.push(node_snapshot(node, latest, &settings, now));
        }
        Ok(snapshots)
    }

    pub fn history(&self, node_id: Uuid, minutes: u64) -> Result<Option<Vec<HistoryPoint>>> {
        let connection = self.lock()?;
        let exists = connection
            .query_row(
                "SELECT 1 FROM nodes WHERE id = ?1",
                [node_id.to_string()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Ok(None);
        }

        let cutoff = Utc::now() - chrono::Duration::minutes(minutes as i64);
        let mut statement = connection.prepare(
            "SELECT received_at, cpu_percent, memory_used_bytes, memory_total_bytes,
                    disk_used_bytes, disk_total_bytes, network_received_bytes_per_sec,
                    network_transmitted_bytes_per_sec, hub_latency_ms, temperature_celsius
             FROM metrics
             WHERE node_id = ?1 AND received_at >= ?2
             ORDER BY received_at ASC, id ASC",
        )?;
        let rows = statement.query_map(params![node_id.to_string(), timestamp(cutoff)], |row| {
            Ok(HistoryPoint {
                collected_at: parse_timestamp(row.get::<_, String>(0)?).map_err(sql_conversion)?,
                cpu_percent: row.get(1)?,
                memory_used_bytes: from_i64(row.get(2)?).map_err(sql_conversion)?,
                memory_total_bytes: from_i64(row.get(3)?).map_err(sql_conversion)?,
                disk_used_bytes: from_i64(row.get(4)?).map_err(sql_conversion)?,
                disk_total_bytes: from_i64(row.get(5)?).map_err(sql_conversion)?,
                network_received_bytes_per_sec: from_i64(row.get(6)?).map_err(sql_conversion)?,
                network_transmitted_bytes_per_sec: from_i64(row.get(7)?).map_err(sql_conversion)?,
                hub_latency_ms: row.get(8)?,
                temperature_celsius: row.get(9)?,
            })
        })?;
        let points = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(downsample_history(points, MAX_HISTORY_POINTS)))
    }

    pub fn alerts(&self) -> Result<Vec<AlertRecord>> {
        let connection = self.lock()?;
        let mut statement = connection.prepare(
            "SELECT id, node_id, node_name, kind, message, value, threshold,
                    active, opened_at, resolved_at, subject_id
             FROM alerts ORDER BY opened_at DESC, id DESC LIMIT 500",
        )?;
        let alerts = statement
            .query_map([], map_alert)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(alerts)
    }

    pub fn active_alert_count(&self) -> Result<usize> {
        let connection = self.lock()?;
        let count: i64 =
            connection.query_row("SELECT COUNT(*) FROM alerts WHERE active = 1", [], |row| {
                row.get(0)
            })?;
        usize::try_from(count).context("active alert count cannot fit in usize")
    }

    pub fn delete_node(&self, node_id: Uuid) -> Result<bool> {
        let connection = self.lock()?;
        let affected =
            connection.execute("DELETE FROM nodes WHERE id = ?1", [node_id.to_string()])?;
        Ok(affected != 0)
    }

    pub fn groups(&self) -> Result<Vec<HostGroup>> {
        let connection = self.lock()?;
        let mut statement = connection
            .prepare("SELECT id, name, created_at FROM groups ORDER BY name COLLATE NOCASE")?;
        statement
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let created_at: String = row.get(2)?;
                Ok(HostGroup {
                    id: Uuid::parse_str(&id).map_err(sql_conversion)?,
                    name: row.get(1)?,
                    created_at: parse_timestamp(created_at).map_err(sql_conversion)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(Into::into)
    }

    pub fn create_group(&self, name: &str) -> Result<HostGroup> {
        let group = HostGroup {
            id: Uuid::new_v4(),
            name: name.to_owned(),
            created_at: Utc::now(),
        };
        let connection = self.lock()?;
        connection.execute(
            "INSERT INTO groups (id, name, created_at) VALUES (?1, ?2, ?3)",
            params![
                group.id.to_string(),
                &group.name,
                timestamp(group.created_at)
            ],
        )?;
        Ok(group)
    }

    pub fn delete_group(&self, group_id: Uuid) -> Result<bool> {
        let connection = self.lock()?;
        let affected =
            connection.execute("DELETE FROM groups WHERE id = ?1", [group_id.to_string()])?;
        Ok(affected != 0)
    }

    pub fn assign_group(
        &self,
        node_id: Uuid,
        group_id: Option<Uuid>,
    ) -> Result<Option<NodeSnapshot>> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        if let Some(group_id) = group_id {
            let exists = transaction
                .query_row(
                    "SELECT 1 FROM groups WHERE id = ?1",
                    [group_id.to_string()],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !exists {
                return Ok(None);
            }
        }
        if transaction.execute(
            "UPDATE nodes SET group_id = ?2 WHERE id = ?1",
            params![node_id.to_string(), group_id.map(|value| value.to_string())],
        )? == 0
        {
            return Ok(None);
        }
        let node =
            get_node_row(&transaction, node_id)?.ok_or_else(|| anyhow!("node disappeared"))?;
        let latest = latest_metric(&transaction, node_id)?;
        let settings = get_settings_from(&transaction)?;
        let snapshot = node_snapshot(node, latest, &settings, Utc::now());
        transaction.commit()?;
        Ok(Some(snapshot))
    }

    pub fn rename_node(&self, node_id: Uuid, display_name: &str) -> Result<Option<NodeSnapshot>> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        if transaction.execute(
            "UPDATE nodes SET display_name = ?2, display_name_overridden = 1 WHERE id = ?1",
            params![node_id.to_string(), display_name],
        )? == 0
        {
            return Ok(None);
        }
        let node =
            get_node_row(&transaction, node_id)?.ok_or_else(|| anyhow!("node disappeared"))?;
        let latest = latest_metric(&transaction, node_id)?;
        let settings = get_settings_from(&transaction)?;
        let snapshot = node_snapshot(node, latest, &settings, Utc::now());
        transaction.commit()?;
        Ok(Some(snapshot))
    }

    pub fn settings(&self) -> Result<AlertSettings> {
        let connection = self.lock()?;
        get_settings_from(&connection)
    }

    pub fn readiness_check(&self) -> Result<()> {
        let connection = self.lock()?;
        connection.query_row("SELECT 1 FROM settings WHERE id = 1", [], |_| Ok(()))?;
        Ok(())
    }

    pub fn update_settings(&self, settings: &AlertSettings) -> Result<()> {
        let connection = self.lock()?;
        connection.execute(
            "UPDATE settings SET value = ?1 WHERE id = 1",
            [serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    pub fn check_offline_nodes(&self) -> Result<Vec<AlertRecord>> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction()?;
        let settings = get_settings_from(&transaction)?;
        if !settings.offline_enabled {
            transaction.commit()?;
            return Ok(Vec::new());
        }
        let now = Utc::now();
        let nodes = {
            let mut statement = transaction.prepare(
                "SELECT nodes.id, hostname, display_name, os, os_version, kernel_version,
                        architecture, agent_version, enrolled_at, last_seen_at, nodes.group_id, groups.name, nodes.browser_latency_url
                 FROM nodes LEFT JOIN groups ON groups.id = nodes.group_id",
            )?;
            statement
                .query_map([], map_node_row)?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let mut alerts = Vec::new();

        for node in nodes {
            let baseline = node.last_seen_at.unwrap_or(node.enrolled_at);
            let offline = now.signed_duration_since(baseline).num_seconds()
                >= settings.offline_after_seconds as i64;
            if offline {
                if !has_active_alert(&transaction, node.id, &AlertKind::Offline)? {
                    let message = format!(
                        "Node has not reported for at least {} seconds",
                        settings.offline_after_seconds
                    );
                    alerts.push(open_alert(
                        &transaction,
                        &node,
                        AlertKind::Offline,
                        message,
                        None,
                        Some(settings.offline_after_seconds as f64),
                        now,
                    )?);
                }
            } else if let Some(resolved) =
                resolve_alert(&transaction, node.id, &AlertKind::Offline, now)?
            {
                alerts.push(resolved);
            }
        }

        transaction.commit()?;
        Ok(alerts)
    }

    pub fn cleanup_old_metrics(&self) -> Result<usize> {
        let connection = self.lock()?;
        let cutoff = Utc::now() - chrono::Duration::days(METRIC_RETENTION_DAYS);
        let deleted = connection.execute(
            "DELETE FROM metrics WHERE received_at < ?1",
            [timestamp(cutoff)],
        )?;
        connection.execute(
            "DELETE FROM probe_samples WHERE received_at < ?1",
            [timestamp(cutoff)],
        )?;
        connection.execute(
            "DELETE FROM service_samples WHERE received_at < ?1",
            [timestamp(cutoff)],
        )?;
        connection.execute(
            "DELETE FROM monitoring_configs WHERE effective_at < ?1 AND revision < (
                SELECT MAX(baseline.revision) FROM monitoring_configs baseline
                WHERE baseline.node_id = monitoring_configs.node_id AND baseline.effective_at < ?1
             )",
            [timestamp(cutoff)],
        )?;
        Ok(deleted)
    }

    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, Connection>> {
        self.connection
            .lock()
            .map_err(|_| anyhow!("database mutex poisoned"))
    }
}

#[derive(Clone)]
struct ThresholdCheck {
    kind: AlertKind,
    value: Option<f64>,
    threshold: f64,
    violated: bool,
    label: &'static str,
    unit: &'static str,
}

fn apply_threshold_check(
    transaction: &Transaction<'_>,
    node: &NodeRow,
    check: ThresholdCheck,
    sustained_for_seconds: u64,
    now: DateTime<Utc>,
) -> Result<Option<AlertRecord>> {
    if !check.violated {
        transaction.execute(
            "DELETE FROM alert_pending WHERE node_id = ?1 AND kind = ?2",
            params![node.id.to_string(), kind_name(&check.kind)],
        )?;
        return resolve_alert(transaction, node.id, &check.kind, now);
    }

    if has_active_alert(transaction, node.id, &check.kind)? {
        return Ok(None);
    }

    let started_at = transaction
        .query_row(
            "SELECT started_at FROM alert_pending WHERE node_id = ?1 AND kind = ?2",
            params![node.id.to_string(), kind_name(&check.kind)],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(parse_timestamp)
        .transpose()?;
    let started_at = match started_at {
        Some(started_at) => started_at,
        None => {
            transaction.execute(
                "INSERT INTO alert_pending (node_id, kind, started_at) VALUES (?1, ?2, ?3)",
                params![node.id.to_string(), kind_name(&check.kind), timestamp(now)],
            )?;
            now
        }
    };

    if now.signed_duration_since(started_at).num_seconds() < sustained_for_seconds as i64 {
        return Ok(None);
    }

    transaction.execute(
        "DELETE FROM alert_pending WHERE node_id = ?1 AND kind = ?2",
        params![node.id.to_string(), kind_name(&check.kind)],
    )?;
    let value = check.value.unwrap_or_default();
    let message = format!(
        "{} is {:.1}{} (threshold {:.1}{})",
        check.label, value, check.unit, check.threshold, check.unit
    );
    Ok(Some(open_alert(
        transaction,
        node,
        check.kind,
        message,
        check.value,
        Some(check.threshold),
        now,
    )?))
}

fn apply_monitoring_alerts(
    transaction: &Transaction<'_>,
    node: &NodeRow,
    report: &MetricReport,
    settings: &AlertSettings,
    now: DateTime<Utc>,
) -> Result<Vec<AlertRecord>> {
    use pinglake_protocol::{MetricStatus, ProbeStatus};
    let Some(data) = &report.monitoring else {
        return Ok(Vec::new());
    };
    let config = crate::monitoring::config_from(transaction, node.id)?;
    let mut checks = Vec::new();
    for service in &data.services {
        if service.config_revision != config.revision {
            continue;
        }
        let Some(target) = config
            .services
            .iter()
            .find(|check| check.id == service.id && check.enabled)
        else {
            continue;
        };
        let healthy = (service.status == MetricStatus::Ok)
            .then_some(service.healthy)
            .flatten();
        checks.push((
            AlertKind::Service,
            service.id,
            healthy,
            service.checked_at,
            data.report_interval_secs.max(5),
            format!(
                "Service {} is {} (expected {})",
                target.name, service.state, target.expected_state
            ),
        ));
    }
    for probe in &data.probes {
        if probe.config_revision != config.revision {
            continue;
        }
        let Some(target) = config
            .probes
            .iter()
            .find(|check| check.id == probe.target_id && check.enabled)
        else {
            continue;
        };
        let healthy = match probe.status {
            ProbeStatus::Success => Some(true),
            ProbeStatus::Failure | ProbeStatus::Timeout => Some(false),
            _ => None,
        };
        checks.push((
            AlertKind::Probe,
            probe.target_id,
            healthy,
            probe.scheduled_at,
            target.interval_secs,
            format!(
                "Probe {} failed: {}",
                target.name,
                probe.error.as_deref().unwrap_or("target unavailable")
            ),
        ));
    }
    checks.sort_by_key(|check| check.3);
    let mut alerts = Vec::new();
    for (kind, subject, healthy, sampled_at, interval, message) in checks {
        let sampled_at = parse_timestamp(timestamp(sampled_at))?;
        // Delayed backlog is persisted for history but cannot change current alert state.
        if sampled_at > now
            || now.signed_duration_since(sampled_at).num_seconds()
                > interval.saturating_mul(2).max(60) as i64
        {
            continue;
        }
        let subject = subject.to_string();
        let previous: Option<(String, Option<String>)> = transaction.query_row(
            "SELECT last_sample_at, pending_since FROM monitoring_check_state WHERE node_id=?1 AND kind=?2 AND subject_id=?3",
            params![node.id.to_string(), kind_name(&kind), subject], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        let mut pending = None;
        if let Some((last_at, pending_since)) = previous {
            let last_at = parse_timestamp(last_at)?;
            if sampled_at <= last_at {
                continue;
            }
            if sampled_at.signed_duration_since(last_at).num_seconds()
                <= interval.saturating_mul(2) as i64
            {
                pending = pending_since.map(parse_timestamp).transpose()?;
            }
        }
        if healthy == Some(false) {
            pending = Some(pending.unwrap_or(sampled_at));
        } else {
            pending = None;
        }
        transaction.execute(
            "INSERT INTO monitoring_check_state(node_id, kind, subject_id, last_sample_at, pending_since) VALUES(?1,?2,?3,?4,?5)
             ON CONFLICT(node_id,kind,subject_id) DO UPDATE SET last_sample_at=excluded.last_sample_at,pending_since=excluded.pending_since",
            params![node.id.to_string(), kind_name(&kind), subject, timestamp(sampled_at), pending.map(timestamp)],
        )?;
        let active: Option<AlertRecord> = transaction.query_row(
            "SELECT id,node_id,node_name,kind,message,value,threshold,active,opened_at,resolved_at,subject_id
             FROM alerts WHERE node_id=?1 AND kind=?2 AND subject_id=?3 AND active=1",
            params![node.id.to_string(), kind_name(&kind), subject], map_alert,
        ).optional()?;
        if healthy == Some(true) {
            if let Some(mut active) = active {
                transaction.execute(
                    "UPDATE alerts SET active=0,resolved_at=?2 WHERE id=?1",
                    params![active.id, timestamp(now)],
                )?;
                active.active = false;
                active.resolved_at = Some(now);
                alerts.push(active);
            }
        } else if healthy == Some(false)
            && active.is_none()
            && pending.is_some_and(|since| {
                sampled_at.signed_duration_since(since).num_seconds()
                    >= settings.sustained_for_seconds as i64
            })
        {
            let node_name = if node.display_name.is_empty() {
                node.hostname.clone()
            } else {
                node.display_name.clone()
            };
            transaction.execute("INSERT INTO alerts(node_id,node_name,kind,subject_id,message,active,opened_at) VALUES(?1,?2,?3,?4,?5,1,?6)",
                params![node.id.to_string(), node_name, kind_name(&kind), subject, message, timestamp(now)])?;
            alerts.push(AlertRecord {
                id: transaction.last_insert_rowid(),
                node_id: node.id,
                node_name,
                kind,
                subject_id: Some(subject),
                message,
                value: None,
                threshold: None,
                active: true,
                opened_at: now,
                resolved_at: None,
            });
        }
    }
    Ok(alerts)
}

fn insert_metric(
    transaction: &Transaction<'_>,
    node_id: Uuid,
    report: &MetricReport,
    received_at: DateTime<Utc>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO metrics (
            node_id, collected_at, received_at, cpu_percent,
            memory_used_bytes, memory_total_bytes, swap_used_bytes, swap_total_bytes,
            disk_used_bytes, disk_total_bytes, network_received_bytes_per_sec,
            network_transmitted_bytes_per_sec, load_one, load_five, load_fifteen,
            temperature_celsius, hub_latency_ms, uptime_seconds, process_count, processes_json, disks_json, interfaces_json,
            monitoring_json, monitoring_session, monitoring_sequence
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25
         )",
        params![
            node_id.to_string(),
            timestamp(report.collected_at),
            timestamp(received_at),
            report.cpu_percent,
            to_i64(report.memory_used_bytes)?,
            to_i64(report.memory_total_bytes)?,
            to_i64(report.swap_used_bytes)?,
            to_i64(report.swap_total_bytes)?,
            to_i64(report.disk_used_bytes)?,
            to_i64(report.disk_total_bytes)?,
            to_i64(report.network_received_bytes_per_sec)?,
            to_i64(report.network_transmitted_bytes_per_sec)?,
            report.load_one,
            report.load_five,
            report.load_fifteen,
            report.temperature_celsius,
            report.hub_latency_ms,
            to_i64(report.uptime_seconds)?,
            i64::try_from(report.process_count).context("process count exceeds SQLite range")?,
            serde_json::to_string(&report.processes)?,
            serde_json::to_string(&report.disks)?,
            serde_json::to_string(&report.interfaces)?,
            report.monitoring.as_ref().map(serde_json::to_string).transpose()?,
            report.monitoring.as_ref().filter(|data| !data.session_id.is_nil()).map(|data| data.session_id.to_string()),
            report.monitoring.as_ref().filter(|data| !data.session_id.is_nil()).map(|data| data.sample_sequence.to_string()),
        ],
    )?;
    Ok(())
}

fn latest_metric(connection: &Connection, node_id: Uuid) -> Result<Option<MetricReport>> {
    connection
        .query_row(
            "SELECT collected_at, cpu_percent, memory_used_bytes, memory_total_bytes,
                    swap_used_bytes, swap_total_bytes, disk_used_bytes, disk_total_bytes,
                    network_received_bytes_per_sec, network_transmitted_bytes_per_sec,
                    load_one, load_five, load_fifteen, temperature_celsius, hub_latency_ms, uptime_seconds,
                    process_count, processes_json, disks_json, interfaces_json, monitoring_json
             FROM metrics WHERE node_id = ?1 ORDER BY id DESC LIMIT 1",
            [node_id.to_string()],
            |row| {
                let processes_json: String = row.get(17)?;
                let disks_json: String = row.get(18)?;
                let interfaces_json: String = row.get(19)?;
                Ok(MetricReport {
                    collected_at: parse_timestamp(row.get::<_, String>(0)?)
                        .map_err(sql_conversion)?,
                    cpu_percent: row.get(1)?,
                    memory_used_bytes: from_i64(row.get(2)?).map_err(sql_conversion)?,
                    memory_total_bytes: from_i64(row.get(3)?).map_err(sql_conversion)?,
                    swap_used_bytes: from_i64(row.get(4)?).map_err(sql_conversion)?,
                    swap_total_bytes: from_i64(row.get(5)?).map_err(sql_conversion)?,
                    disk_used_bytes: from_i64(row.get(6)?).map_err(sql_conversion)?,
                    disk_total_bytes: from_i64(row.get(7)?).map_err(sql_conversion)?,
                    network_received_bytes_per_sec: from_i64(row.get(8)?)
                        .map_err(sql_conversion)?,
                    network_transmitted_bytes_per_sec: from_i64(row.get(9)?)
                        .map_err(sql_conversion)?,
                    load_one: row.get(10)?,
                    load_five: row.get(11)?,
                    load_fifteen: row.get(12)?,
                    temperature_celsius: row.get(13)?,
                    hub_latency_ms: row.get(14)?,
                    uptime_seconds: from_i64(row.get(15)?).map_err(sql_conversion)?,
                    process_count: usize::try_from(row.get::<_, i64>(16)?)
                        .map_err(sql_conversion)?,
                    processes: serde_json::from_str(&processes_json).map_err(sql_conversion)?,
                    disks: serde_json::from_str(&disks_json).map_err(sql_conversion)?,
                    interfaces: serde_json::from_str(&interfaces_json).map_err(sql_conversion)?,
                    monitoring: row.get::<_, Option<String>>(20)?.map(|json| serde_json::from_str(&json).map_err(sql_conversion)).transpose()?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn get_node_row(transaction: &Transaction<'_>, node_id: Uuid) -> Result<Option<NodeRow>> {
    transaction
        .query_row(
            "SELECT nodes.id, hostname, display_name, os, os_version, kernel_version,
                    architecture, agent_version, enrolled_at, last_seen_at, nodes.group_id, groups.name, nodes.browser_latency_url
             FROM nodes LEFT JOIN groups ON groups.id = nodes.group_id WHERE nodes.id = ?1",
            [node_id.to_string()],
            map_node_row,
        )
        .optional()
        .map_err(Into::into)
}

fn map_node_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<NodeRow> {
    let id_text: String = row.get(0)?;
    let enrolled_at: String = row.get(8)?;
    let last_seen_at: Option<String> = row.get(9)?;
    let group_id: Option<String> = row.get(10)?;
    Ok(NodeRow {
        id: Uuid::parse_str(&id_text).map_err(sql_conversion)?,
        hostname: row.get(1)?,
        display_name: row.get(2)?,
        os: row.get(3)?,
        os_version: row.get(4)?,
        kernel_version: row.get(5)?,
        architecture: row.get(6)?,
        agent_version: row.get(7)?,
        group_id: group_id
            .map(|value| Uuid::parse_str(&value))
            .transpose()
            .map_err(sql_conversion)?,
        group_name: row.get(11)?,
        browser_latency_url: row.get(12)?,
        enrolled_at: parse_timestamp(enrolled_at).map_err(sql_conversion)?,
        last_seen_at: last_seen_at
            .map(parse_timestamp)
            .transpose()
            .map_err(sql_conversion)?,
    })
}

fn node_snapshot(
    node: NodeRow,
    latest: Option<MetricReport>,
    settings: &AlertSettings,
    now: DateTime<Utc>,
) -> NodeSnapshot {
    let baseline = node.last_seen_at.unwrap_or(node.enrolled_at);
    let online = node.last_seen_at.is_some()
        && now.signed_duration_since(baseline).num_seconds()
            < settings.offline_after_seconds as i64;
    NodeSnapshot {
        id: node.id,
        hostname: node.hostname,
        display_name: node.display_name,
        os: node.os,
        os_version: node.os_version,
        kernel_version: node.kernel_version,
        architecture: node.architecture,
        agent_version: node.agent_version,
        group_id: node.group_id,
        group_name: node.group_name,
        browser_latency_url: node.browser_latency_url,
        enrolled_at: node.enrolled_at,
        last_seen_at: node.last_seen_at,
        online,
        latest,
    }
}

fn open_alert(
    transaction: &Transaction<'_>,
    node: &NodeRow,
    kind: AlertKind,
    message: String,
    value: Option<f64>,
    threshold: Option<f64>,
    now: DateTime<Utc>,
) -> Result<AlertRecord> {
    let node_name = if node.display_name.trim().is_empty() {
        node.hostname.clone()
    } else {
        node.display_name.clone()
    };
    transaction.execute(
        "INSERT INTO alerts (
            node_id, node_name, kind, message, value, threshold, active, opened_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
        params![
            node.id.to_string(),
            node_name,
            kind_name(&kind),
            message,
            value,
            threshold,
            timestamp(now),
        ],
    )?;
    let id = transaction.last_insert_rowid();
    Ok(AlertRecord {
        id,
        node_id: node.id,
        node_name,
        kind,
        message,
        value,
        threshold,
        active: true,
        opened_at: now,
        resolved_at: None,
        subject_id: None,
    })
}

fn resolve_alert(
    transaction: &Transaction<'_>,
    node_id: Uuid,
    kind: &AlertKind,
    now: DateTime<Utc>,
) -> Result<Option<AlertRecord>> {
    let alert = transaction
        .query_row(
            "SELECT id, node_id, node_name, kind, message, value, threshold,
                    active, opened_at, resolved_at, subject_id
             FROM alerts WHERE node_id = ?1 AND kind = ?2 AND active = 1 LIMIT 1",
            params![node_id.to_string(), kind_name(kind)],
            map_alert,
        )
        .optional()?;
    let Some(mut alert) = alert else {
        return Ok(None);
    };
    transaction.execute(
        "UPDATE alerts SET active = 0, resolved_at = ?2 WHERE id = ?1",
        params![alert.id, timestamp(now)],
    )?;
    alert.active = false;
    alert.resolved_at = Some(now);
    Ok(Some(alert))
}

fn has_active_alert(
    transaction: &Transaction<'_>,
    node_id: Uuid,
    kind: &AlertKind,
) -> Result<bool> {
    Ok(transaction
        .query_row(
            "SELECT 1 FROM alerts WHERE node_id = ?1 AND kind = ?2 AND active = 1 LIMIT 1",
            params![node_id.to_string(), kind_name(kind)],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn map_alert(row: &rusqlite::Row<'_>) -> rusqlite::Result<AlertRecord> {
    let node_id: String = row.get(1)?;
    let kind: String = row.get(3)?;
    let opened_at: String = row.get(8)?;
    let resolved_at: Option<String> = row.get(9)?;
    let subject_id: String = row.get(10)?;
    Ok(AlertRecord {
        id: row.get(0)?,
        node_id: Uuid::parse_str(&node_id).map_err(sql_conversion)?,
        node_name: row.get(2)?,
        kind: parse_kind(&kind).map_err(sql_conversion)?,
        message: row.get(4)?,
        value: row.get(5)?,
        threshold: row.get(6)?,
        active: row.get::<_, i64>(7)? != 0,
        opened_at: parse_timestamp(opened_at).map_err(sql_conversion)?,
        resolved_at: resolved_at
            .map(parse_timestamp)
            .transpose()
            .map_err(sql_conversion)?,
        subject_id: (!subject_id.is_empty()).then_some(subject_id),
    })
}

fn get_settings_from(connection: &Connection) -> Result<AlertSettings> {
    let value: String =
        connection.query_row("SELECT value FROM settings WHERE id = 1", [], |row| {
            row.get(0)
        })?;
    serde_json::from_str(&value).context("invalid alert settings in database")
}

fn migrate(connection: &Connection) -> Result<()> {
    let mut version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > 5 {
        bail!("database schema version {version} is newer than this Hub supports");
    }
    if version == 0 {
        let default_settings = serde_json::to_string(&AlertSettings::default())?;
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(
            "CREATE TABLE nodes (
                id TEXT PRIMARY KEY NOT NULL,
                secret_hash TEXT NOT NULL,
                hostname TEXT NOT NULL,
                display_name TEXT NOT NULL,
                display_name_overridden INTEGER NOT NULL DEFAULT 0,
                os TEXT NOT NULL,
                os_version TEXT NOT NULL,
                kernel_version TEXT NOT NULL,
                architecture TEXT NOT NULL,
                agent_version TEXT NOT NULL,
                group_id TEXT REFERENCES groups(id) ON DELETE SET NULL,
                enrolled_at TEXT NOT NULL,
                last_seen_at TEXT
             );
             CREATE TABLE groups (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                created_at TEXT NOT NULL
             );
             CREATE TABLE metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                collected_at TEXT NOT NULL,
                received_at TEXT NOT NULL,
                cpu_percent REAL NOT NULL,
                memory_used_bytes INTEGER NOT NULL,
                memory_total_bytes INTEGER NOT NULL,
                swap_used_bytes INTEGER NOT NULL,
                swap_total_bytes INTEGER NOT NULL,
                disk_used_bytes INTEGER NOT NULL,
                disk_total_bytes INTEGER NOT NULL,
                network_received_bytes_per_sec INTEGER NOT NULL,
                network_transmitted_bytes_per_sec INTEGER NOT NULL,
                load_one REAL,
                load_five REAL,
                load_fifteen REAL,
                temperature_celsius REAL,
                hub_latency_ms REAL,
                uptime_seconds INTEGER NOT NULL,
                process_count INTEGER NOT NULL,
                processes_json TEXT NOT NULL DEFAULT '[]',
                disks_json TEXT NOT NULL,
                interfaces_json TEXT NOT NULL
             );
             CREATE TABLE alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                node_name TEXT NOT NULL,
                kind TEXT NOT NULL,
                message TEXT NOT NULL,
                value REAL,
                threshold REAL,
                active INTEGER NOT NULL,
                opened_at TEXT NOT NULL,
                resolved_at TEXT
             );
             CREATE TABLE alert_pending (
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                started_at TEXT NOT NULL,
                PRIMARY KEY (node_id, kind)
             );
             CREATE TABLE settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                value TEXT NOT NULL
             );
             CREATE INDEX metrics_node_received_idx
                ON metrics(node_id, received_at DESC);
             CREATE INDEX metrics_received_idx ON metrics(received_at);
             CREATE INDEX alerts_opened_idx ON alerts(opened_at DESC);
             CREATE UNIQUE INDEX alerts_one_active_kind_idx
                ON alerts(node_id, kind) WHERE active = 1;
             ",
        )?;
        transaction.execute(
            "INSERT INTO settings (id, value) VALUES (1, ?1)",
            [default_settings],
        )?;
        transaction.pragma_update(None, "user_version", 4)?;
        transaction.commit()?;
        version = 4;
    }
    if version == 1 {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(
            "ALTER TABLE metrics ADD COLUMN processes_json TEXT NOT NULL DEFAULT '[]';",
        )?;
        transaction.pragma_update(None, "user_version", 2)?;
        transaction.commit()?;
        version = 2;
    }
    if version == 2 {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(
            "CREATE TABLE groups (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL COLLATE NOCASE UNIQUE,
                created_at TEXT NOT NULL
             );
             ALTER TABLE nodes ADD COLUMN group_id TEXT REFERENCES groups(id) ON DELETE SET NULL;",
        )?;
        transaction.pragma_update(None, "user_version", 3)?;
        transaction.commit()?;
        version = 3;
    }
    if version == 3 {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(
            "ALTER TABLE nodes ADD COLUMN display_name_overridden INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE metrics ADD COLUMN hub_latency_ms REAL;",
        )?;
        transaction.pragma_update(None, "user_version", 4)?;
        transaction.commit()?;
        version = 4;
    }
    if version == 4 {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(
            "ALTER TABLE nodes ADD COLUMN browser_latency_url TEXT;
             ALTER TABLE alerts ADD COLUMN subject_id TEXT NOT NULL DEFAULT '';
             DROP INDEX alerts_one_active_kind_idx;
             CREATE UNIQUE INDEX alerts_one_active_kind_idx ON alerts(node_id, kind, subject_id) WHERE active = 1;
             ALTER TABLE metrics ADD COLUMN monitoring_json TEXT;
             ALTER TABLE metrics ADD COLUMN monitoring_session TEXT;
             ALTER TABLE metrics ADD COLUMN monitoring_sequence TEXT;
             CREATE UNIQUE INDEX metrics_monitoring_sample_idx
               ON metrics(node_id, monitoring_session, monitoring_sequence) WHERE monitoring_session IS NOT NULL;
             CREATE TABLE monitoring_configs (
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                revision INTEGER NOT NULL,
                effective_at TEXT NOT NULL,
                config_json TEXT NOT NULL,
                PRIMARY KEY(node_id, revision)
             );
             CREATE INDEX monitoring_configs_time_idx ON monitoring_configs(node_id, effective_at);
             CREATE TABLE probe_samples (
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                sample_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                config_revision INTEGER NOT NULL,
                scheduled_at TEXT NOT NULL,
                received_at TEXT NOT NULL,
                result_json TEXT NOT NULL,
                PRIMARY KEY(node_id, sample_id)
             );
             CREATE INDEX probe_samples_target_time_idx ON probe_samples(node_id, target_id, scheduled_at);
             CREATE INDEX probe_samples_received_idx ON probe_samples(node_id, received_at DESC);
             CREATE INDEX probe_samples_retention_idx ON probe_samples(received_at);
             CREATE TABLE service_samples (
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                subject_id TEXT NOT NULL,
                config_revision INTEGER NOT NULL,
                checked_at TEXT NOT NULL,
                received_at TEXT NOT NULL,
                result_json TEXT NOT NULL,
                PRIMARY KEY(node_id, subject_id, config_revision, checked_at)
             );
             CREATE INDEX service_samples_time_idx ON service_samples(node_id, checked_at);
             CREATE INDEX service_samples_received_idx ON service_samples(node_id, received_at DESC);
             CREATE INDEX service_samples_retention_idx ON service_samples(received_at);
             CREATE TABLE monitoring_check_state (
                node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
                kind TEXT NOT NULL,
                subject_id TEXT NOT NULL,
                last_sample_at TEXT NOT NULL,
                pending_since TEXT,
                PRIMARY KEY(node_id, kind, subject_id)
             );"
        )?;
        transaction.pragma_update(None, "user_version", 5)?;
        transaction.commit()?;
    }
    Ok(())
}

fn percent(used: u64, total: u64) -> Option<f64> {
    (total != 0).then(|| used as f64 * 100.0 / total as f64)
}

fn downsample_history(points: Vec<HistoryPoint>, maximum_points: usize) -> Vec<HistoryPoint> {
    if maximum_points == 0 || points.len() <= maximum_points {
        return points;
    }

    let point_count = points.len();
    (0..maximum_points)
        .map(|bucket| {
            let start = bucket * point_count / maximum_points;
            let end = ((bucket + 1) * point_count / maximum_points).max(start + 1);
            aggregate_history_bucket(&points[start..end])
        })
        .collect()
}

fn aggregate_history_bucket(points: &[HistoryPoint]) -> HistoryPoint {
    debug_assert!(!points.is_empty());
    let latest = points.last().expect("non-empty history bucket");
    HistoryPoint {
        collected_at: latest.collected_at,
        cpu_percent: average_f32(points.iter().map(|point| point.cpu_percent)),
        memory_used_bytes: average_u64(points.iter().map(|point| point.memory_used_bytes)),
        memory_total_bytes: average_u64(points.iter().map(|point| point.memory_total_bytes)),
        disk_used_bytes: average_u64(points.iter().map(|point| point.disk_used_bytes)),
        disk_total_bytes: average_u64(points.iter().map(|point| point.disk_total_bytes)),
        network_received_bytes_per_sec: average_u64(
            points
                .iter()
                .map(|point| point.network_received_bytes_per_sec),
        ),
        network_transmitted_bytes_per_sec: average_u64(
            points
                .iter()
                .map(|point| point.network_transmitted_bytes_per_sec),
        ),
        hub_latency_ms: average_optional_f32(
            points.iter().filter_map(|point| point.hub_latency_ms),
        ),
        temperature_celsius: points
            .iter()
            .filter_map(|point| point.temperature_celsius)
            .max_by(f32::total_cmp),
    }
}

fn average_optional_f32(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, count) = values.fold((0.0_f64, 0_u64), |(sum, count), value| {
        (sum + f64::from(value), count.saturating_add(1))
    });
    (count != 0).then(|| (sum / count as f64) as f32)
}

fn average_u64(values: impl Iterator<Item = u64>) -> u64 {
    let (sum, count) = values.fold((0_u128, 0_u128), |(sum, count), value| {
        (sum + u128::from(value), count + 1)
    });
    u64::try_from(sum / count).expect("average of u64 values fits in u64")
}

fn average_f32(values: impl Iterator<Item = f32>) -> f32 {
    let (sum, count) = values.fold((0.0_f64, 0_u64), |(sum, count), value| {
        (sum + f64::from(value), count + 1)
    });
    (sum / count as f64) as f32
}

fn timestamp(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_timestamp(value: String) -> Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(&value)
        .with_context(|| format!("invalid database timestamp {value:?}"))?
        .with_timezone(&Utc))
}

fn kind_name(kind: &AlertKind) -> &'static str {
    match kind {
        AlertKind::Offline => "offline",
        AlertKind::Cpu => "cpu",
        AlertKind::Memory => "memory",
        AlertKind::Disk => "disk",
        AlertKind::Temperature => "temperature",
        AlertKind::Service => "service",
        AlertKind::Probe => "probe",
    }
}

fn parse_kind(value: &str) -> Result<AlertKind> {
    match value {
        "offline" => Ok(AlertKind::Offline),
        "cpu" => Ok(AlertKind::Cpu),
        "memory" => Ok(AlertKind::Memory),
        "disk" => Ok(AlertKind::Disk),
        "temperature" => Ok(AlertKind::Temperature),
        "service" => Ok(AlertKind::Service),
        "probe" => Ok(AlertKind::Probe),
        _ => bail!("invalid alert kind in database"),
    }
}

fn to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("metric value exceeds SQLite integer range")
}

fn from_i64(value: i64) -> Result<u64> {
    u64::try_from(value).context("negative metric value in database")
}

pub(crate) fn constant_time_equal(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.as_bytes()
        .iter()
        .zip(right.as_bytes())
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn sql_conversion(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, error.into())
}

#[cfg(test)]
mod tests {
    use chrono::Duration;

    use super::*;

    #[test]
    fn v4_migration_preserves_existing_data_and_scopes_alert_uniqueness() {
        let connection = Connection::open_in_memory().unwrap();
        connection.execute_batch("CREATE TABLE nodes(id TEXT PRIMARY KEY);
            CREATE TABLE metrics(id INTEGER PRIMARY KEY, node_id TEXT, received_at TEXT);
            CREATE TABLE alerts(id INTEGER PRIMARY KEY, node_id TEXT, kind TEXT, active INTEGER);
            CREATE UNIQUE INDEX alerts_one_active_kind_idx ON alerts(node_id, kind) WHERE active=1;
            INSERT INTO nodes(id) VALUES('existing');
            INSERT INTO metrics(id,node_id,received_at) VALUES(1,'existing','2026-01-01T00:00:00.000Z');
            INSERT INTO alerts(id,node_id,kind,active) VALUES(1,'existing','cpu',1);
            PRAGMA user_version=4;").unwrap();
        migrate(&connection).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            5
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM metrics", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT subject_id FROM alerts WHERE id=1", [], |row| row
                    .get::<_, String>(
                    0
                ))
                .unwrap(),
            ""
        );
        connection.execute("INSERT INTO alerts(node_id,kind,active,subject_id) VALUES('existing','service',1,'one')", []).unwrap();
        connection.execute("INSERT INTO alerts(node_id,kind,active,subject_id) VALUES('existing','service',1,'two')", []).unwrap();
        assert!(connection.execute("INSERT INTO alerts(node_id,kind,active,subject_id) VALUES('existing','service',1,'one')", []).is_err());
        migrate(&connection).unwrap();
        connection.pragma_update(None, "user_version", 6).unwrap();
        assert!(migrate(&connection).is_err());
    }

    #[test]
    fn history_downsampling_bounds_output_and_keeps_latest_sample() {
        let started_at = Utc::now();
        let points = (0_u64..3_001)
            .map(|index| HistoryPoint {
                collected_at: started_at + Duration::seconds(index as i64),
                cpu_percent: index as f32,
                memory_used_bytes: index,
                memory_total_bytes: 10_000,
                disk_used_bytes: index,
                disk_total_bytes: 20_000,
                network_received_bytes_per_sec: index,
                network_transmitted_bytes_per_sec: index,
                hub_latency_ms: Some(index as f32),
                temperature_celsius: Some(index as f32),
            })
            .collect::<Vec<_>>();
        let expected_last = points.last().expect("point").collected_at;

        let sampled = downsample_history(points, 1_440);

        assert_eq!(sampled.len(), 1_440);
        assert_eq!(sampled.last().expect("sample").collected_at, expected_last);
        assert!(
            sampled
                .windows(2)
                .all(|window| { window[0].collected_at <= window[1].collected_at })
        );
    }

    #[test]
    fn short_history_is_not_modified() {
        let point = HistoryPoint {
            collected_at: Utc::now(),
            cpu_percent: 50.0,
            memory_used_bytes: 1,
            memory_total_bytes: 2,
            disk_used_bytes: 1,
            disk_total_bytes: 2,
            network_received_bytes_per_sec: 1,
            network_transmitted_bytes_per_sec: 1,
            hub_latency_ms: Some(1.0),
            temperature_celsius: None,
        };
        let points = vec![point.clone()];

        let sampled = downsample_history(points, 1_440);

        assert_eq!(sampled.len(), 1);
        assert_eq!(sampled[0].cpu_percent, point.cpu_percent);
    }
}
