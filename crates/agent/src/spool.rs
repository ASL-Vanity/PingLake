use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use pinglake_protocol::MetricReport;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_CAPACITY: usize = 64;
pub const MAX_CAPACITY: usize = 4096;
pub const DEFAULT_TTL: Duration = Duration::from_secs(300);
pub const MAX_TTL: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Clone)]
pub struct SpoolItem {
    pub id: i64,
    pub report: MetricReport,
    pub queued_at: DateTime<Utc>,
}

pub struct Spool {
    path: PathBuf,
    capacity: usize,
    ttl: Duration,
}

impl Spool {
    pub fn open(path: impl Into<PathBuf>, agent_id: &str, hub: &str) -> Result<Self> {
        let path = path.into();
        let capacity = DEFAULT_CAPACITY;
        let ttl = DEFAULT_TTL;
        let result = Self::open_inner(&path, agent_id, hub, capacity, ttl);
        match result {
            Ok(spool) => Ok(spool),
            Err(error) => {
                if error.to_string().contains("binding changed") {
                    return Err(error);
                }
                let stamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let corrupt = path.with_extension(format!("corrupt-{stamp}"));
                if path.exists() {
                    let _ = fs::rename(&path, &corrupt);
                }
                tracing::error!(path=%path.display(), quarantine=%corrupt.display(), reason=%error, "agent spool was damaged; starting empty");
                Self::open_inner(&path, agent_id, hub, capacity, ttl)
            }
        }
    }
    fn open_inner(
        path: &Path,
        agent_id: &str,
        hub: &str,
        capacity: usize,
        ttl: Duration,
    ) -> Result<Self> {
        if capacity == 0 || capacity > MAX_CAPACITY || ttl.is_zero() || ttl > MAX_TTL {
            bail!("invalid spool limits")
        }
        let conn =
            Connection::open(path).with_context(|| format!("open spool {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);\
            CREATE TABLE IF NOT EXISTS reports (id INTEGER PRIMARY KEY AUTOINCREMENT, queued_at TEXT NOT NULL, state TEXT NOT NULL, payload BLOB NOT NULL);\
            CREATE INDEX IF NOT EXISTS reports_ready ON reports(state, id);")?;
        let existing: Option<String> = conn
            .query_row("SELECT value FROM meta WHERE key='binding'", [], |r| {
                r.get(0)
            })
            .optional()?;
        let binding = format!("{agent_id}|{hub}");
        if let Some(value) = existing {
            if value != binding {
                bail!("spool identity or hub binding changed")
            }
        } else {
            conn.execute(
                "INSERT INTO meta(key,value) VALUES('binding',?1)",
                [binding],
            )?;
        }
        drop(conn);
        Ok(Self {
            path: path.to_owned(),
            capacity,
            ttl,
        })
    }
    fn conn(&self) -> Result<Connection> {
        Ok(Connection::open(&self.path)?)
    }
    pub fn restore(&self) -> Result<Vec<SpoolItem>> {
        let conn = self.conn()?;
        let cutoff = Utc::now() - chrono::Duration::from_std(self.ttl)?;
        conn.execute(
            "DELETE FROM reports WHERE queued_at < ?1",
            [cutoff.to_rfc3339()],
        )?;
        let mut stmt = conn.prepare("SELECT id, queued_at, payload FROM reports ORDER BY CASE state WHEN 'inflight' THEN 0 ELSE 1 END, id")?;
        let rows = stmt.query_map([], |r| {
            let at: String = r.get(1)?;
            let payload: Vec<u8> = r.get(2)?;
            Ok((r.get::<_, i64>(0)?, at, payload))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, at, bytes) = row?;
            let report = serde_json::from_slice(&bytes).context("invalid spool report")?;
            out.push(SpoolItem {
                id,
                report,
                queued_at: DateTime::parse_from_rfc3339(&at)?.with_timezone(&Utc),
            });
        }
        conn.execute(
            "UPDATE reports SET state='pending' WHERE state='inflight'",
            [],
        )?;
        Ok(out)
    }
    pub fn enqueue(&self, report: &MetricReport, queued_at: DateTime<Utc>) -> Result<(i64, u64)> {
        let conn = self.conn()?;
        let bytes = serde_json::to_vec(report)?;
        let tx = conn.unchecked_transaction()?;
        let count: i64 = tx.query_row("SELECT count(*) FROM reports", [], |r| r.get(0))?;
        let mut dropped = 0;
        if count >= self.capacity as i64 {
            tx.execute(
                "DELETE FROM reports WHERE id=(SELECT id FROM reports ORDER BY id LIMIT 1)",
                [],
            )?;
            dropped = 1;
        }
        tx.execute(
            "INSERT INTO reports(queued_at,state,payload) VALUES(?1,'pending',?2)",
            params![queued_at.to_rfc3339(), bytes],
        )?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok((id, dropped))
    }
    pub fn claim(&self, id: i64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute(
            "UPDATE reports SET state='inflight' WHERE id=?1 AND state='pending'",
            [id],
        )?;
        Ok(())
    }
    pub fn ack(&self, id: i64) -> Result<()> {
        let conn = self.conn()?;
        conn.execute("DELETE FROM reports WHERE id=?1", [id])?;
        Ok(())
    }
    pub fn pending(&self) -> Result<usize> {
        let conn = self.conn()?;
        Ok(conn.query_row("SELECT count(*) FROM reports", [], |r| r.get::<_, i64>(0))? as usize)
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn replace_all(&self, reports: &[(MetricReport, DateTime<Utc>)]) -> Result<()> {
        let conn = self.conn()?;
        let tx = conn.unchecked_transaction()?;
        tx.execute("DELETE FROM reports", [])?;
        for (report, at) in reports {
            tx.execute(
                "INSERT INTO reports(queued_at,state,payload) VALUES(?1,'pending',?2)",
                params![at.to_rfc3339(), serde_json::to_vec(report)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn report() -> MetricReport {
        MetricReport {
            collected_at: Utc::now(),
            cpu_percent: 1.0,
            memory_used_bytes: 1,
            memory_total_bytes: 2,
            swap_used_bytes: 0,
            swap_total_bytes: 0,
            disk_used_bytes: 0,
            disk_total_bytes: 1,
            network_received_bytes_per_sec: 0,
            network_transmitted_bytes_per_sec: 0,
            hub_latency_ms: None,
            load_one: None,
            load_five: None,
            load_fifteen: None,
            temperature_celsius: None,
            uptime_seconds: 1,
            process_count: 0,
            processes: vec![],
            disks: vec![],
            interfaces: vec![],
            monitoring: None,
        }
    }

    #[test]
    fn restart_restores_fifo_and_preserves_queued_at() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("spool.sqlite");
        let spool = Spool::open(&path, "agent", "hub").unwrap();
        let at = Utc::now() - chrono::Duration::seconds(7);
        spool.enqueue(&report(), at).unwrap();
        spool
            .enqueue(&report(), at + chrono::Duration::seconds(1))
            .unwrap();
        let restored = Spool::open(&path, "agent", "hub")
            .unwrap()
            .restore()
            .unwrap();
        assert_eq!(restored.len(), 2);
        assert_eq!(restored[0].queued_at, at);
    }

    #[test]
    fn binding_mismatch_is_rejected() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("spool.sqlite");
        Spool::open(&path, "agent", "hub-a").unwrap();
        assert!(Spool::open(&path, "agent", "hub-b").is_err());
    }
}
