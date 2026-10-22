use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};

use crate::settings::Settings;
use crate::util::{iso_timestamp, now_wall};

#[derive(Clone, Debug, Serialize)]
pub struct SessionRecord {
    pub id: String,
    pub boot_id: String,
    pub root_pid: u32,
    pub root_start_ticks: u64,
    pub browser: String,
    pub executable: String,
    pub command: String,
    pub cgroup: Option<String>,
    pub uid: Option<u32>,
    pub launched_at: f64,
    pub first_seen_at: f64,
    pub last_seen_at: f64,
    pub discovery_mode: String,
    pub source: Value,
    pub ancestry: Value,
    pub parent: Value,
    pub status: Value,
    pub peak_cpu: Option<f64>,
    pub latest_rss: u64,
    pub latest_pss: Option<u64>,
    pub latest_process_count: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistorySession {
    pub id: String,
    pub boot_id: String,
    pub root_pid: u32,
    pub root_start_ticks: u64,
    pub browser: String,
    pub executable: String,
    pub command: String,
    pub cgroup: Option<String>,
    pub uid: Option<u32>,
    pub launched_at: f64,
    pub first_seen_at: f64,
    pub last_seen_at: f64,
    pub ended_at: Option<f64>,
    pub discovery_mode: String,
    pub source: Value,
    pub ancestry: Value,
    pub parent: Value,
    pub status: Value,
    pub peak_cpu: Option<f64>,
    pub latest_rss: u64,
    pub latest_pss: Option<u64>,
    pub latest_process_count: usize,
    pub final_view: Option<Value>,
}

#[derive(Clone, Debug, Serialize)]
pub struct EventRow {
    pub id: i64,
    pub t: f64,
    pub at: Option<String>,
    pub session_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub severity: String,
    pub message: String,
    pub evidence: Value,
    pub acknowledged: bool,
    pub muted_until: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionRow {
    pub id: i64,
    pub t: f64,
    pub at: Option<String>,
    pub session_id: String,
    pub action: String,
    pub targets: usize,
    pub result: String,
    pub message: String,
    pub evidence: Value,
}

#[derive(Clone, Debug)]
pub struct MetricSample {
    pub session_id: String,
    pub ts: f64,
    pub cpu_cores: Option<f64>,
    pub rss_sum: u64,
    pub pss_sum: Option<u64>,
    pub process_count: usize,
    pub quality: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricPoint {
    pub t: f64,
    pub cpu_cores: f64,
    pub rss_sum: u64,
    pub pss_sum: Option<u64>,
    pub process_count: usize,
    pub quality: String,
}

pub struct Storage {
    conn: Connection,
    pub path: PathBuf,
}

fn json_or_empty(value: &str) -> Value {
    serde_json::from_str(value).unwrap_or_else(|_| Value::Object(Default::default()))
}

impl Storage {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("cannot create data directory {}", data_dir.display()))?;
        fs::set_permissions(data_dir, fs::Permissions::from_mode(0o700)).ok();
        let path = data_dir.join("headless-watch.db");
        let conn = Connection::open(&path)
            .with_context(|| format!("cannot open SQLite database {}", path.display()))?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).ok();
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        conn.execute_batch(
            "PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             CREATE TABLE IF NOT EXISTS sessions (
               id TEXT PRIMARY KEY,
               boot_id TEXT NOT NULL,
               root_pid INTEGER NOT NULL,
               root_start_ticks INTEGER NOT NULL,
               browser TEXT NOT NULL,
               executable TEXT NOT NULL,
               command TEXT NOT NULL,
               cgroup TEXT,
               uid INTEGER,
               launched_at REAL NOT NULL,
               first_seen_at REAL NOT NULL,
               last_seen_at REAL NOT NULL,
               ended_at REAL,
               discovery_mode TEXT NOT NULL,
               source_json TEXT NOT NULL,
               ancestry_json TEXT NOT NULL,
               parent_json TEXT NOT NULL,
               status_json TEXT NOT NULL,
               peak_cpu REAL,
               latest_rss INTEGER NOT NULL DEFAULT 0,
               latest_pss INTEGER,
               latest_process_count INTEGER NOT NULL DEFAULT 0,
               final_json TEXT
             );
             CREATE INDEX IF NOT EXISTS sessions_ended ON sessions(ended_at DESC);
             CREATE TABLE IF NOT EXISTS metric_samples (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL,
               ts REAL NOT NULL,
               cpu_cores REAL,
               rss_sum INTEGER NOT NULL,
               pss_sum INTEGER,
               process_count INTEGER NOT NULL,
               quality TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS metrics_session_ts ON metric_samples(session_id, ts);
             CREATE TABLE IF NOT EXISTS metric_rollups (
               session_id TEXT NOT NULL,
               bucket_ts INTEGER NOT NULL,
               cpu_sum REAL NOT NULL DEFAULT 0,
               cpu_n INTEGER NOT NULL DEFAULT 0,
               rss_sum INTEGER NOT NULL DEFAULT 0,
               rss_n INTEGER NOT NULL DEFAULT 0,
               pss_sum INTEGER NOT NULL DEFAULT 0,
               pss_n INTEGER NOT NULL DEFAULT 0,
               process_max INTEGER NOT NULL DEFAULT 0,
               PRIMARY KEY(session_id, bucket_ts)
             );
             CREATE INDEX IF NOT EXISTS rollups_ts ON metric_rollups(bucket_ts);
             CREATE TRIGGER IF NOT EXISTS metric_rollup_ai AFTER INSERT ON metric_samples
             BEGIN
               INSERT INTO metric_rollups(
                 session_id,bucket_ts,cpu_sum,cpu_n,rss_sum,rss_n,pss_sum,pss_n,process_max
               ) VALUES (
                 NEW.session_id,
                 CAST(NEW.ts / 60 AS INTEGER) * 60,
                 COALESCE(NEW.cpu_cores, 0),
                 CASE WHEN NEW.cpu_cores IS NULL THEN 0 ELSE 1 END,
                 NEW.rss_sum, 1,
                 COALESCE(NEW.pss_sum, 0),
                 CASE WHEN NEW.pss_sum IS NULL THEN 0 ELSE 1 END,
                 NEW.process_count
               )
               ON CONFLICT(session_id,bucket_ts) DO UPDATE SET
                 cpu_sum=cpu_sum+excluded.cpu_sum,
                 cpu_n=cpu_n+excluded.cpu_n,
                 rss_sum=rss_sum+excluded.rss_sum,
                 rss_n=rss_n+excluded.rss_n,
                 pss_sum=pss_sum+excluded.pss_sum,
                 pss_n=pss_n+excluded.pss_n,
                 process_max=MAX(process_max,excluded.process_max);
             END;
             CREATE TABLE IF NOT EXISTS events (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT,
               ts REAL NOT NULL,
               kind TEXT NOT NULL,
               severity TEXT NOT NULL,
               message TEXT NOT NULL,
               evidence_json TEXT NOT NULL,
               acknowledged INTEGER NOT NULL DEFAULT 0,
               muted_until REAL
             );
             CREATE INDEX IF NOT EXISTS events_ts ON events(ts DESC);
             CREATE TABLE IF NOT EXISTS actions (
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               session_id TEXT NOT NULL,
               ts REAL NOT NULL,
               action TEXT NOT NULL,
               targets INTEGER NOT NULL,
               result TEXT NOT NULL,
               message TEXT NOT NULL,
               evidence_json TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS actions_ts ON actions(ts DESC);
             CREATE TABLE IF NOT EXISTS settings (
               key TEXT PRIMARY KEY,
               value TEXT NOT NULL
             );
             PRAGMA user_version=1;",
        )?;
        Ok(Self { conn, path })
    }

    pub fn load_settings(&self) -> Result<Settings> {
        let raw: Option<String> = self
            .conn
            .query_row("SELECT value FROM settings WHERE key='app'", [], |row| {
                row.get(0)
            })
            .optional()?;
        if let Some(raw) = raw {
            if let Ok(settings) = serde_json::from_str::<Settings>(&raw) {
                return Ok(settings.normalized());
            }
        }
        let settings = Settings::default();
        self.save_settings(&settings)?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings(key,value) VALUES('app',?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![serde_json::to_string(settings)?],
        )?;
        Ok(())
    }

    pub fn upsert_session(&self, record: &SessionRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions(
               id,boot_id,root_pid,root_start_ticks,browser,executable,command,cgroup,uid,
               launched_at,first_seen_at,last_seen_at,ended_at,discovery_mode,source_json,
               ancestry_json,parent_json,status_json,peak_cpu,latest_rss,latest_pss,
               latest_process_count,final_json
             ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,NULL,?13,?14,?15,?16,?17,?18,?19,?20,?21,NULL)
             ON CONFLICT(id) DO UPDATE SET
               last_seen_at=excluded.last_seen_at,
               ended_at=NULL,
               final_json=NULL,
               root_pid=excluded.root_pid,
               root_start_ticks=excluded.root_start_ticks,
               browser=excluded.browser,
               executable=excluded.executable,
               command=excluded.command,
               cgroup=excluded.cgroup,
               uid=excluded.uid,
               discovery_mode=excluded.discovery_mode,
               first_seen_at=MIN(sessions.first_seen_at, excluded.first_seen_at),
               status_json=excluded.status_json,
               peak_cpu=excluded.peak_cpu,
               latest_rss=excluded.latest_rss,
               latest_pss=excluded.latest_pss,
               latest_process_count=excluded.latest_process_count",
            params![
                record.id,
                record.boot_id,
                record.root_pid,
                record.root_start_ticks as i64,
                record.browser,
                record.executable,
                record.command,
                record.cgroup,
                record.uid,
                record.launched_at,
                record.first_seen_at,
                record.last_seen_at,
                record.discovery_mode,
                record.source.to_string(),
                record.ancestry.to_string(),
                record.parent.to_string(),
                record.status.to_string(),
                record.peak_cpu,
                record.latest_rss as i64,
                record.latest_pss.map(|value| value as i64),
                record.latest_process_count as i64,
            ],
        )?;
        Ok(())
    }

    pub fn end_session(&self, id: &str, ended_at: f64, final_view: &Value) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET ended_at=?2, last_seen_at=MAX(last_seen_at,?2), final_json=?3 WHERE id=?1",
            params![id, ended_at, final_view.to_string()],
        )?;
        Ok(())
    }

    pub fn load_open_session(&self, id: &str) -> Result<Option<HistorySession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,boot_id,root_pid,root_start_ticks,browser,executable,command,cgroup,uid,
                    launched_at,first_seen_at,last_seen_at,ended_at,discovery_mode,source_json,
                    ancestry_json,parent_json,status_json,peak_cpu,latest_rss,latest_pss,
                    latest_process_count,final_json
             FROM sessions WHERE id=?1 AND ended_at IS NULL",
        )?;
        let row = stmt.query_row(params![id], map_history_row).optional()?;
        Ok(row)
    }

    pub fn reconcile_open_sessions(
        &self,
        active_ids: &std::collections::HashSet<String>,
    ) -> Result<usize> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM sessions WHERE ended_at IS NULL")?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let now = now_wall();
        let mut ended = 0;
        for id in ids {
            if !active_ids.contains(&id) {
                self.conn.execute(
                    "UPDATE sessions SET ended_at=?2, status_json=?3 WHERE id=?1 AND ended_at IS NULL",
                    params![id, now, "{\"lifecycle\":\"ended\",\"reason\":\"backend_restart\"}"],
                )?;
                ended += 1;
            }
        }
        Ok(ended)
    }

    pub fn insert_metric(
        &self,
        session_id: &str,
        ts: f64,
        cpu_cores: Option<f64>,
        rss_sum: u64,
        pss_sum: Option<u64>,
        process_count: usize,
        quality: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO metric_samples(session_id,ts,cpu_cores,rss_sum,pss_sum,process_count,quality)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                session_id,
                ts,
                cpu_cores,
                rss_sum as i64,
                pss_sum.map(|value| value as i64),
                process_count as i64,
                quality
            ],
        )?;
        Ok(())
    }

    pub fn insert_metrics_batch(&self, samples: &[MetricSample]) -> Result<()> {
        if samples.is_empty() {
            return Ok(());
        }
        let transaction = self.conn.unchecked_transaction()?;
        {
            let mut statement = transaction.prepare(
                "INSERT INTO metric_samples(session_id,ts,cpu_cores,rss_sum,pss_sum,process_count,quality)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
            )?;
            for sample in samples {
                statement.execute(params![
                    sample.session_id,
                    sample.ts,
                    sample.cpu_cores,
                    sample.rss_sum as i64,
                    sample.pss_sum.map(|value| value as i64),
                    sample.process_count as i64,
                    sample.quality,
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn insert_event(
        &self,
        session_id: Option<&str>,
        ts: f64,
        kind: &str,
        severity: &str,
        message: &str,
        evidence: &Value,
    ) -> Result<EventRow> {
        self.conn.execute(
            "INSERT INTO events(session_id,ts,kind,severity,message,evidence_json)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![
                session_id,
                ts,
                kind,
                severity,
                message,
                evidence.to_string()
            ],
        )?;
        Ok(EventRow {
            id: self.conn.last_insert_rowid(),
            t: ts,
            at: iso_timestamp(Some(ts)),
            session_id: session_id.map(str::to_string),
            kind: kind.to_string(),
            severity: severity.to_string(),
            message: message.to_string(),
            evidence: evidence.clone(),
            acknowledged: false,
            muted_until: None,
        })
    }

    pub fn recent_events(&self, limit: usize) -> Result<Vec<EventRow>> {
        self.events_since(None, limit, None)
    }

    pub fn events_since(
        &self,
        since: Option<f64>,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,ts,session_id,kind,severity,message,evidence_json,acknowledged,muted_until
             FROM events
             WHERE (?1 IS NULL OR ts >= ?1)
               AND (?2 IS NULL OR session_id = ?2)
             ORDER BY ts DESC, id DESC LIMIT ?3",
        )?;
        let rows = stmt
            .query_map(params![since, session_id, limit.min(5000) as i64], |row| {
                let evidence: String = row.get(6)?;
                let ts: f64 = row.get(1)?;
                Ok(EventRow {
                    id: row.get(0)?,
                    t: ts,
                    at: iso_timestamp(Some(ts)),
                    session_id: row.get(2)?,
                    kind: row.get(3)?,
                    severity: row.get(4)?,
                    message: row.get(5)?,
                    evidence: json_or_empty(&evidence),
                    acknowledged: row.get::<_, i64>(7)? != 0,
                    muted_until: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_alerts(&self, limit: usize) -> Result<Vec<EventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,ts,session_id,kind,severity,message,evidence_json,acknowledged,muted_until
             FROM events
             WHERE kind IN ('high_cpu','suspected_leftover','action_failed')
             ORDER BY ts DESC, id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.min(500) as i64], |row| {
                let evidence: String = row.get(6)?;
                let ts: f64 = row.get(1)?;
                Ok(EventRow {
                    id: row.get(0)?,
                    t: ts,
                    at: iso_timestamp(Some(ts)),
                    session_id: row.get(2)?,
                    kind: row.get(3)?,
                    severity: row.get(4)?,
                    message: row.get(5)?,
                    evidence: json_or_empty(&evidence),
                    acknowledged: row.get::<_, i64>(7)? != 0,
                    muted_until: row.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn acknowledge_alert(&self, id: i64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE events SET acknowledged=1 WHERE id=?1 AND kind IN ('high_cpu','suspected_leftover','action_failed')",
            params![id],
        )?;
        Ok(changed > 0)
    }

    pub fn mute_alert(&self, id: i64, until: f64) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE events SET muted_until=?2 WHERE id=?1 AND kind IN ('high_cpu','suspected_leftover','action_failed')",
            params![id, until],
        )?;
        Ok(changed > 0)
    }

    pub fn insert_action(
        &self,
        session_id: &str,
        ts: f64,
        action: &str,
        targets: usize,
        result: &str,
        message: &str,
        evidence: &Value,
    ) -> Result<ActionRow> {
        self.conn.execute(
            "INSERT INTO actions(session_id,ts,action,targets,result,message,evidence_json)
             VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                session_id,
                ts,
                action,
                targets as i64,
                result,
                message,
                evidence.to_string()
            ],
        )?;
        Ok(ActionRow {
            id: self.conn.last_insert_rowid(),
            t: ts,
            at: iso_timestamp(Some(ts)),
            session_id: session_id.to_string(),
            action: action.to_string(),
            targets,
            result: result.to_string(),
            message: message.to_string(),
            evidence: evidence.clone(),
        })
    }

    pub fn recent_actions(&self, limit: usize) -> Result<Vec<ActionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,ts,session_id,action,targets,result,message,evidence_json
             FROM actions ORDER BY ts DESC,id DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.min(1000) as i64], |row| {
                let evidence: String = row.get(7)?;
                let ts: f64 = row.get(1)?;
                Ok(ActionRow {
                    id: row.get(0)?,
                    t: ts,
                    at: iso_timestamp(Some(ts)),
                    session_id: row.get(2)?,
                    action: row.get(3)?,
                    targets: row.get::<_, i64>(4)?.max(0) as usize,
                    result: row.get(5)?,
                    message: row.get(6)?,
                    evidence: json_or_empty(&evidence),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_ended_sessions(&self, limit: usize) -> Result<Vec<HistorySession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,boot_id,root_pid,root_start_ticks,browser,executable,command,cgroup,uid,
                    launched_at,first_seen_at,last_seen_at,ended_at,discovery_mode,source_json,
                    ancestry_json,parent_json,status_json,peak_cpu,latest_rss,latest_pss,
                    latest_process_count,final_json
             FROM sessions WHERE ended_at IS NOT NULL ORDER BY ended_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.min(5000) as i64], map_history_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_ended_summaries(&self, limit: usize) -> Result<Vec<Value>> {
        let rows = self.list_ended_sessions(limit)?;
        Ok(rows
            .into_iter()
            .map(|row| {
                let status = row
                    .final_view
                    .as_ref()
                    .and_then(|view| view.get("status"))
                    .cloned()
                    .unwrap_or(row.status);
                let source = row
                    .final_view
                    .as_ref()
                    .and_then(|view| view.get("source"))
                    .cloned()
                    .unwrap_or(row.source);
                json!({
                    "id": row.id,
                    "pid": row.root_pid,
                    "browser": row.browser,
                    "executable": row.executable,
                    "discovery_mode": row.discovery_mode,
                    "launched_at": row.launched_at,
                    "ended_at": row.ended_at,
                    "peak_cpu": row.peak_cpu,
                    "latest_rss": row.latest_rss,
                    "latest_pss": row.latest_pss,
                    "latest_process_count": row.latest_process_count,
                    "source": source,
                    "status": status,
                })
            })
            .collect())
    }

    pub fn get_history_session(&self, id: &str) -> Result<Option<HistorySession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,boot_id,root_pid,root_start_ticks,browser,executable,command,cgroup,uid,
                    launched_at,first_seen_at,last_seen_at,ended_at,discovery_mode,source_json,
                    ancestry_json,parent_json,status_json,peak_cpu,latest_rss,latest_pss,
                    latest_process_count,final_json
             FROM sessions WHERE id=?1",
        )?;
        Ok(stmt.query_row(params![id], map_history_row).optional()?)
    }

    pub fn list_all_sessions(&self, limit: usize) -> Result<Vec<HistorySession>> {
        let mut stmt = self.conn.prepare(
            "SELECT id,boot_id,root_pid,root_start_ticks,browser,executable,command,cgroup,uid,
                    launched_at,first_seen_at,last_seen_at,ended_at,discovery_mode,source_json,
                    ancestry_json,parent_json,status_json,peak_cpu,latest_rss,latest_pss,
                    latest_process_count,final_json
             FROM sessions ORDER BY first_seen_at DESC LIMIT ?1",
        )?;
        let rows = stmt
            .query_map(params![limit.min(10000) as i64], map_history_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn metrics(
        &self,
        session_id: &str,
        window_s: f64,
        max_points: usize,
        raw_retention_hours: u32,
    ) -> Result<Vec<MetricPoint>> {
        let now = now_wall();
        let since = now - window_s.max(60.0);
        let raw_cutoff = now - raw_retention_hours.max(1) as f64 * 3600.0;
        let bucket = (window_s / max_points.max(60) as f64).max(1.0);
        let mut points = std::collections::BTreeMap::<i64, MetricPoint>::new();
        {
            let mut stmt = self.conn.prepare(
                "SELECT (CAST(ts / ?2 AS INTEGER) * ?2) AS bucket,
                        AVG(cpu_cores), AVG(rss_sum), AVG(pss_sum), MAX(process_count)
                 FROM metric_samples
                 WHERE session_id=?1 AND ts>=?3 AND ts>=?4
                 GROUP BY bucket ORDER BY bucket ASC",
            )?;
            let rows = stmt.query_map(params![session_id, bucket, since, raw_cutoff], |row| {
                Ok(MetricPoint {
                    t: row.get(0)?,
                    cpu_cores: row.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                    rss_sum: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0).max(0.0) as u64,
                    pss_sum: row
                        .get::<_, Option<f64>>(3)?
                        .map(|value| value.max(0.0) as u64),
                    process_count: row.get::<_, i64>(4)?.max(0) as usize,
                    quality: "raw".to_string(),
                })
            })?;
            for point in rows {
                let point = point?;
                points.insert(point.t as i64, point);
            }
        }
        if since < raw_cutoff {
            let mut stmt = self.conn.prepare(
                "SELECT (CAST(bucket_ts / ?2 AS INTEGER) * ?2) AS bucket,
                        CASE WHEN SUM(cpu_n)>0 THEN SUM(cpu_sum)/SUM(cpu_n) ELSE 0 END,
                        CASE WHEN SUM(rss_n)>0 THEN SUM(rss_sum)/SUM(rss_n) ELSE 0 END,
                        CASE WHEN SUM(pss_n)>0 THEN CAST(SUM(pss_sum) AS REAL)/SUM(pss_n) ELSE NULL END,
                        MAX(process_max)
                 FROM metric_rollups
                 WHERE session_id=?1 AND bucket_ts>=?3 AND bucket_ts<?4
                 GROUP BY bucket ORDER BY bucket ASC",
            )?;
            let rows = stmt.query_map(params![session_id, bucket, since, raw_cutoff], |row| {
                Ok(MetricPoint {
                    t: row.get(0)?,
                    cpu_cores: row.get::<_, f64>(1)?.max(0.0),
                    rss_sum: row.get::<_, f64>(2)?.max(0.0) as u64,
                    pss_sum: row
                        .get::<_, Option<f64>>(3)?
                        .map(|value| value.max(0.0) as u64),
                    process_count: row.get::<_, i64>(4)?.max(0) as usize,
                    quality: "rollup".to_string(),
                })
            })?;
            for point in rows {
                let point = point?;
                points.insert(point.t as i64, point);
            }
        }
        Ok(points.into_values().collect())
    }

    pub fn cleanup(&self, settings: &Settings) -> Result<()> {
        let now = now_wall();
        let raw_cutoff = now - settings.retention_raw_hours as f64 * 3600.0;
        let events_cutoff = now - settings.retention_events_days as f64 * 86400.0;
        self.conn.execute(
            "DELETE FROM metric_samples WHERE ts < ?1",
            params![raw_cutoff],
        )?;
        self.conn
            .execute("DELETE FROM events WHERE ts < ?1", params![events_cutoff])?;
        self.conn
            .execute("DELETE FROM actions WHERE ts < ?1", params![events_cutoff])?;
        self.conn.execute(
            "DELETE FROM sessions WHERE ended_at IS NOT NULL AND ended_at < ?1",
            params![events_cutoff],
        )?;
        self.conn.execute(
            "DELETE FROM metric_rollups WHERE bucket_ts < ?1",
            params![now - 7.0 * 86400.0],
        )?;
        Ok(())
    }

    pub fn database_size(&self) -> u64 {
        fs::metadata(&self.path).map(|meta| meta.len()).unwrap_or(0)
    }
}

fn map_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HistorySession> {
    let source: String = row.get(14)?;
    let ancestry: String = row.get(15)?;
    let parent: String = row.get(16)?;
    let status: String = row.get(17)?;
    let final_raw: Option<String> = row.get(22)?;
    Ok(HistorySession {
        id: row.get(0)?,
        boot_id: row.get(1)?,
        root_pid: row.get::<_, i64>(2)?.max(0) as u32,
        root_start_ticks: row.get::<_, i64>(3)?.max(0) as u64,
        browser: row.get(4)?,
        executable: row.get(5)?,
        command: row.get(6)?,
        cgroup: row.get(7)?,
        uid: row
            .get::<_, Option<i64>>(8)?
            .map(|value| value.max(0) as u32),
        launched_at: row.get(9)?,
        first_seen_at: row.get(10)?,
        last_seen_at: row.get(11)?,
        ended_at: row.get(12)?,
        discovery_mode: row.get(13)?,
        source: json_or_empty(&source),
        ancestry: json_or_empty(&ancestry),
        parent: json_or_empty(&parent),
        status: json_or_empty(&status),
        peak_cpu: row.get(18)?,
        latest_rss: row.get::<_, i64>(19)?.max(0) as u64,
        latest_pss: row
            .get::<_, Option<i64>>(20)?
            .map(|value| value.max(0) as u64),
        latest_process_count: row.get::<_, i64>(21)?.max(0) as usize,
        final_view: final_raw.and_then(|value| serde_json::from_str(&value).ok()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn record(id: &str) -> SessionRecord {
        SessionRecord {
            id: id.to_string(),
            boot_id: "boot".to_string(),
            root_pid: 100,
            root_start_ticks: 50,
            browser: "Google Chrome".to_string(),
            executable: "chrome".to_string(),
            command: "chrome --headless".to_string(),
            cgroup: None,
            uid: Some(1000),
            launched_at: 90.0,
            first_seen_at: 100.0,
            last_seen_at: 101.0,
            discovery_mode: "observed_live".to_string(),
            source: json!({"launcher":"OpenCode"}),
            ancestry: json!([]),
            parent: json!({}),
            status: json!({"lifecycle":"active"}),
            peak_cpu: Some(1.0),
            latest_rss: 1024,
            latest_pss: None,
            latest_process_count: 2,
        }
    }

    #[test]
    fn persists_and_reconciles_sessions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::open(dir.path()).expect("storage");
        let mode = fs::metadata(&storage.path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        storage.upsert_session(&record("s1")).expect("insert");
        assert!(storage.load_open_session("s1").expect("load").is_some());
        storage
            .reconcile_open_sessions(&std::collections::HashSet::new())
            .expect("reconcile");
        assert!(storage.load_open_session("s1").expect("load").is_none());
        let ended = storage.list_ended_sessions(10).expect("ended");
        assert_eq!(ended.len(), 1);
        assert_eq!(ended[0].source["launcher"], "OpenCode");
    }

    #[test]
    fn aggregates_old_metrics_into_rollups() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::open(dir.path()).expect("storage");
        let now = now_wall();
        storage
            .insert_metric("s1", now - 7200.0, Some(1.5), 4096, Some(2048), 3, "ok")
            .expect("metric");
        let rollup = storage
            .metrics("s1", 86400.0, 360, 1)
            .expect("rollup metrics");
        assert_eq!(rollup.len(), 1);
        assert_eq!(rollup[0].quality, "rollup");
        assert!((rollup[0].cpu_cores - 1.5).abs() < 0.001);
        assert_eq!(rollup[0].pss_sum, Some(2048));
        storage
            .insert_metric("s1", now, Some(0.5), 1024, None, 2, "ok")
            .expect("metric");
        let raw = storage.metrics("s1", 3600.0, 60, 1).expect("raw metrics");
        assert!(raw.iter().any(|point| point.quality == "raw"));
    }

    #[test]
    fn persists_events_and_actions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage = Storage::open(dir.path()).expect("storage");
        let event = storage
            .insert_event(Some("s1"), 100.0, "high_cpu", "warning", "hot", &json!({}))
            .expect("event");
        assert!(storage.acknowledge_alert(event.id).expect("ack"));
        assert!(storage.list_alerts(10).expect("alerts")[0].acknowledged);
        storage
            .insert_action("s1", 101.0, "term", 3, "stopped", "done", &json!({}))
            .expect("action");
        assert_eq!(storage.recent_actions(10).expect("actions").len(), 1);
    }
}
