use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use tracing::warn;
use uuid::Uuid;

use crate::actions::TerminationOutcome;
use crate::cdp::CdpProbe;
use crate::model::VERSION;
use crate::notify;
use crate::procfs::{read_boot_id, scan, DEFAULT_PROC_ROOT};
use crate::redaction::redact_text;
use crate::settings::Settings;
use crate::storage::{EventRow, Storage};
use crate::tracker::{overview, Notify, Tracker};
use crate::util::{iso_timestamp, now_wall};

pub struct PssResult {
    pub session_id: String,
    pub values: Vec<(u32, u64)>,
    pub errors: Vec<String>,
    pub sampled_at: f64,
}

pub struct Core {
    pub tracker: Tracker,
    pub storage: Storage,
    pub settings: Settings,
    pub boot_id: String,
    pub token: String,
    pub revision: u64,
    pub web_dir: PathBuf,
    pub proc_root: PathBuf,
    pub started_wall: f64,
    pub notifications_available: bool,
    last_cleanup: Instant,
}

impl Core {
    pub fn open(
        data_dir: &Path,
        web_dir: PathBuf,
        proc_root: Option<PathBuf>,
        mut settings_override: Option<Settings>,
    ) -> Result<Self> {
        let storage = Storage::open(data_dir)?;
        let mut settings = storage.load_settings()?;
        if let Some(override_settings) = settings_override.take() {
            settings = override_settings.normalized();
            storage.save_settings(&settings)?;
        }
        let proc_root = proc_root.unwrap_or_else(|| PathBuf::from(DEFAULT_PROC_ROOT));
        let boot_id = read_boot_id(&proc_root);
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let online_cpus = crate::procfs::online_cpu_count();
        let mut tracker = Tracker::new(boot_id.clone(), online_cpus);
        tracker.load_recent(&storage);
        let started_wall = now_wall();
        let core = Self {
            tracker,
            storage,
            settings,
            boot_id,
            token,
            revision: 0,
            web_dir,
            proc_root,
            started_wall,
            notifications_available: notify::available(),
            last_cleanup: Instant::now() - Duration::from_secs(600),
        };
        Ok(core)
    }

    pub fn initialize(&mut self) -> Vec<Notify> {
        let notices = self.scan_once();
        let active = self.tracker.active_ids();
        if let Err(err) = self.storage.reconcile_open_sessions(&active) {
            warn!("failed to reconcile persisted sessions: {err}");
        }
        self.cleanup_if_due();
        notices
    }

    pub fn scan_once(&mut self) -> Vec<Notify> {
        let snapshot = scan(&self.proc_root, &self.boot_id);
        self.revision = self.revision.saturating_add(1);
        let notices = self
            .tracker
            .update(snapshot, &self.settings, &mut self.storage);
        self.cleanup_if_due();
        notices
    }

    pub fn snapshot(&self) -> Value {
        overview(&self.tracker, &self.settings, &self.storage, self.revision)
    }

    pub fn history_session(&self, id: &str) -> Result<Option<Value>> {
        Ok(self
            .storage
            .get_history_session(id)?
            .map(|session| crate::tracker::history_biography(&session)))
    }

    pub fn settings_json(&self) -> Value {
        json!({
            "settings": self.settings,
            "notifications_available": self.notifications_available,
            "database_size": self.storage.database_size(),
            "version": VERSION,
            "data_path": self.storage.path,
        })
    }

    pub fn update_settings(&mut self, settings: Settings) -> Result<(), String> {
        settings.validate()?;
        self.settings = settings.normalized();
        self.storage
            .save_settings(&self.settings)
            .map_err(|err| format!("Cannot persist settings: {err}"))?;
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn apply_pss_results(&mut self, results: Vec<PssResult>) {
        for result in results {
            self.tracker.apply_pss(
                &result.session_id,
                result.values,
                result.errors,
                result.sampled_at,
            );
            let Some(stats) = self.tracker.pss_stats(&result.session_id) else {
                continue;
            };
            if let Err(err) = self.storage.insert_metric(
                &result.session_id,
                result.sampled_at,
                stats.cpu_cores,
                stats.rss_sum,
                stats.pss_sum,
                stats.process_count,
                &stats.quality,
            ) {
                warn!("failed to persist PSS sample: {err}");
            }
        }
        if self.tracker.has_sessions() {
            self.revision = self.revision.saturating_add(1);
        }
    }

    pub fn apply_cdp_results(&mut self, results: Vec<(String, u16, CdpProbe, f64)>) {
        if results.is_empty() {
            return;
        }
        for (session_id, port, probe, checked_at) in results {
            self.tracker.apply_cdp(&session_id, port, probe, checked_at);
        }
        self.revision = self.revision.saturating_add(1);
    }

    pub fn pss_targets(&self) -> Vec<(String, Vec<(u32, u64)>)> {
        self.tracker.pss_targets()
    }

    pub fn cdp_targets(&self) -> Vec<(String, u16)> {
        self.tracker.cdp_targets()
    }

    pub fn mark_terminating(&mut self, id: &str, mode: &str) -> bool {
        let Some((_, _, _, _, blocked)) = self.tracker.termination_info(id) else {
            return false;
        };
        if blocked {
            return false;
        }
        self.tracker.mark_terminating(id, mode);
        self.revision = self.revision.saturating_add(1);
        true
    }

    pub fn finish_termination(&mut self, id: &str, message: &str, still_alive: bool) {
        self.tracker.finish_termination(id, message, still_alive);
        self.revision = self.revision.saturating_add(1);
    }

    pub fn termination_info(&self, id: &str) -> Option<(String, u32, u64, Option<u32>, bool)> {
        self.tracker.termination_info(id)
    }

    pub fn record_action(
        &mut self,
        session_id: &str,
        action: &str,
        outcome: &TerminationOutcome,
        evidence: Value,
    ) -> Option<Notify> {
        let ts = now_wall();
        let message = redact_text(&outcome.message, 240);
        let row = match self.storage.insert_action(
            session_id,
            ts,
            action,
            outcome.targets,
            &outcome.result,
            &message,
            &evidence,
        ) {
            Ok(row) => row,
            Err(err) => {
                warn!("failed to persist action: {err}");
                return None;
            }
        };
        self.tracker.actions.insert(0, row);
        self.tracker.actions.truncate(100);
        let failed = !outcome.ok;
        let kind = if failed { "action_failed" } else { "action" };
        let severity = if failed { "warning" } else { "info" };
        if let Ok(event) = self.storage.insert_event(
            Some(session_id),
            ts,
            kind,
            severity,
            &message,
            &json!({
                "action": action,
                "targets": outcome.targets,
                "signaled": outcome.signaled,
                "survivors": outcome.survivors,
                "unauthorized": outcome.unauthorized,
            }),
        ) {
            self.tracker.events.insert(0, event);
            self.tracker.events.truncate(200);
        }
        self.revision = self.revision.saturating_add(1);
        (failed && self.settings.notifications && !self.settings.global_mute).then(|| Notify {
            title: "Headless Watch action failed".to_string(),
            body: message,
        })
    }

    pub fn record_rejected_action(
        &mut self,
        session_id: &str,
        action: &str,
        message: &str,
    ) -> Option<Notify> {
        let ts = now_wall();
        let message = redact_text(message, 240);
        if let Ok(row) =
            self.storage
                .insert_action(session_id, ts, action, 0, "rejected", &message, &json!({}))
        {
            self.tracker.actions.insert(0, row);
            self.tracker.actions.truncate(100);
        }
        if let Ok(event) = self.storage.insert_event(
            Some(session_id),
            ts,
            "action_failed",
            "warning",
            &message,
            &json!({"action": action, "rejected": true}),
        ) {
            self.tracker.events.insert(0, event);
            self.tracker.events.truncate(200);
        }
        self.revision = self.revision.saturating_add(1);
        (self.settings.notifications && !self.settings.global_mute).then(|| Notify {
            title: "Headless Watch action rejected".to_string(),
            body: message,
        })
    }

    pub fn acknowledge_alert(&mut self, id: i64) -> Result<bool> {
        let changed = self.storage.acknowledge_alert(id)?;
        if changed {
            if let Some(event) = self.tracker.events.iter_mut().find(|event| event.id == id) {
                event.acknowledged = true;
            }
            self.revision = self.revision.saturating_add(1);
        }
        Ok(changed)
    }

    pub fn mute_alert(&mut self, id: i64, minutes: u64) -> Result<bool> {
        let until = now_wall() + minutes.min(24 * 60) as f64 * 60.0;
        let changed = self.storage.mute_alert(id, until)?;
        if changed {
            if let Some(event) = self.tracker.events.iter_mut().find(|event| event.id == id) {
                event.muted_until = Some(until);
            }
            self.revision = self.revision.saturating_add(1);
        }
        Ok(changed)
    }

    pub fn mute_session(&mut self, id: &str, minutes: u64) -> bool {
        let changed = self.tracker.mute_session(id, minutes);
        if changed {
            self.revision = self.revision.saturating_add(1);
        }
        changed
    }

    pub fn alert_list(&self, limit: usize) -> Result<Vec<EventRow>> {
        self.storage.list_alerts(limit)
    }

    fn cleanup_if_due(&mut self) {
        if self.last_cleanup.elapsed() >= Duration::from_secs(600) {
            if let Err(err) = self.storage.cleanup(&self.settings) {
                warn!("history cleanup failed: {err}");
            }
            self.last_cleanup = Instant::now();
        }
    }

    pub fn export_json(&self) -> Result<Value> {
        let sessions = self.storage.list_all_sessions(10000)?;
        let events = self.storage.events_since(None, 5000, None)?;
        let actions = self.storage.recent_actions(1000)?;
        Ok(json!({
            "version": VERSION,
            "exported_at": iso_timestamp(Some(now_wall())),
            "sessions": sessions,
            "events": events,
            "actions": actions,
        }))
    }
}

pub fn resolve_web_dir(explicit: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(explicit) = explicit {
        return explicit
            .canonicalize()
            .with_context(|| format!("web directory does not exist: {}", explicit.display()));
    }
    if let Ok(value) = std::env::var("HEADLESS_WATCH_WEB_DIR") {
        let path = PathBuf::from(value);
        if path.join("index.html").is_file() {
            return path
                .canonicalize()
                .context("cannot resolve HEADLESS_WATCH_WEB_DIR");
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join("web"));
            candidates.push(parent.join("frontend/dist"));
        }
    }
    candidates.push(PathBuf::from("frontend/dist"));
    candidates.push(PathBuf::from("web"));
    for candidate in candidates {
        if candidate.join("index.html").is_file() {
            return candidate
                .canonicalize()
                .context("cannot resolve dashboard web directory");
        }
    }
    anyhow::bail!("dashboard assets not found; run `pnpm --dir frontend build` or pass --web-dir")
}
