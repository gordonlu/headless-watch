use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::attribution::{build_attribution, own_command, Attribution};
use crate::browser::{browser_roots, cdp_endpoint, process_role, CdpEndpoint};
use crate::cdp::{CdpProbe, CdpTab};
use crate::model::{machine_pct, CpuValue, ProcSnapshot, ProcessIdentity, ProcessInfo, VERSION};
use crate::procfs::{descendants, identity_alive};
use crate::redaction::{redact_argv, redact_path, redact_text};
use crate::settings::Settings;
use crate::storage::{ActionRow, EventRow, MetricSample, SessionRecord, Storage};
use crate::util::{format_duration, iso_timestamp, now_wall};

const ACTIVE_LIMIT: usize = 500;

#[derive(Clone, Debug, Serialize)]
pub struct CpuView {
    pub cores: Option<f64>,
    pub single_core_pct: Option<f64>,
    pub machine_pct: Option<f64>,
    pub peak_cores: Option<f64>,
    pub quality: String,
    pub online_cpus: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct PssCoverage {
    pub sampled: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryView {
    pub rss_sum: u64,
    pub rss_quality: String,
    pub pss_sum: Option<u64>,
    pub pss_quality: String,
    pub pss_age_s: Option<f64>,
    pub pss_coverage: Option<PssCoverage>,
    pub pss_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusView {
    pub lifecycle: String,
    pub stale: bool,
    pub parent_exited: bool,
    pub parent_state: String,
    pub launcher_exited: bool,
    pub launcher_exited_at: Option<f64>,
    pub driver_exited: bool,
    pub driver_exited_at: Option<f64>,
    pub watch_exited_at: Option<f64>,
    pub suspected_leftover: bool,
    pub suspected_at: Option<f64>,
    pub high_cpu: bool,
    pub high_cpu_at: Option<f64>,
    pub terminating: bool,
    pub risk: String,
    pub muted_until: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SourceView {
    pub summary: String,
    pub confidence: String,
    pub launcher: String,
    pub launcher_confidence: String,
    pub launcher_pid: Option<u32>,
    pub launcher_name: Option<String>,
    pub driver: String,
    pub driver_confidence: String,
    pub driver_pid: Option<u32>,
    pub driver_name: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AncestorView {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub command: String,
    pub role: Option<String>,
    pub start_ticks: u64,
    pub alive: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct ParentView {
    pub pid: Option<u32>,
    pub name: Option<String>,
    pub observable: bool,
    pub state: String,
    pub exited: bool,
    pub exited_at: Option<f64>,
    pub note: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProcessView {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub role: String,
    pub command: String,
    pub uid: Option<u32>,
    pub rss_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct CdpView {
    pub mode: String,
    pub state: String,
    pub port: Option<u16>,
    pub reason: Option<String>,
    pub tabs: Vec<CdpTab>,
    pub screenshot_available: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct HistoryPoint {
    pub t: f64,
    pub cores: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SessionView {
    pub id: String,
    pub pid: u32,
    pub start_ticks: u64,
    pub identity: ProcessIdentity,
    pub browser: String,
    pub executable: String,
    pub command: String,
    pub cgroup: Option<String>,
    pub uid: Option<u32>,
    pub discovery_mode: String,
    pub preexisting: bool,
    pub first_seen_at: f64,
    pub launched_at: f64,
    pub ended_at: Option<f64>,
    pub lifetime_s: i64,
    pub source: SourceView,
    pub ancestry: Vec<AncestorView>,
    pub parent: ParentView,
    pub status: StatusView,
    pub cpu: CpuView,
    pub memory: MemoryView,
    pub process_count: usize,
    pub processes: Vec<ProcessView>,
    pub roles: HashMap<String, usize>,
    pub history: Vec<HistoryPoint>,
    pub cdp: CdpView,
    pub can_terminate: bool,
    pub force_available: bool,
    pub action_notice: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SummaryView {
    pub active_sessions: usize,
    pub process_count: usize,
    pub cpu_cores: f64,
    pub cpu_single_core_pct: f64,
    pub cpu_machine_pct: f64,
    pub online_cpus: usize,
    pub calculating_sessions: usize,
    pub rss_sum: u64,
    pub pss_sum: Option<u64>,
    pub pss_age_s: Option<f64>,
    pub parent_exited: usize,
    pub suspected_leftovers: usize,
    pub high_cpu: usize,
    pub unknown_origin: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct HealthView {
    pub platform: &'static str,
    pub pss_supported: bool,
    pub graceful_stop_supported: bool,
    pub status: String,
    pub last_scan_age_s: Option<f64>,
    pub last_scan_at: Option<String>,
    pub scan_failures: u64,
    pub last_error: Option<String>,
    pub stale_sessions: usize,
    pub online_cpus: usize,
    pub monitor_pid: u32,
    pub active_sessions: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SystemPoint {
    pub t: f64,
    pub cores: f64,
    pub machine_pct: f64,
    pub rss_sum: u64,
    pub pss_sum: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct Notify {
    pub title: String,
    pub body: String,
    pub session_id: Option<String>,
}

struct PendingEvent {
    session_id: Option<String>,
    kind: String,
    severity: String,
    message: String,
    evidence: Value,
    notify: Option<Notify>,
}

struct Session {
    id: String,
    root_identity: ProcessIdentity,
    boot_id: String,
    root_pid: u32,
    root_start_ticks: u64,
    browser: String,
    executable: String,
    command: String,
    cgroup: Option<String>,
    uid: Option<u32>,
    discovery_mode: String,
    preexisting: bool,
    first_seen_at: f64,
    launched_at: f64,
    attribution: Attribution,
    online_cpus: usize,
    last_scan_instant: Option<Instant>,
    prev_ticks: HashMap<String, u64>,
    cpu: Option<CpuValue>,
    peak_cpu: Option<f64>,
    cpu_quality: String,
    rss_sum: u64,
    pss_sum: Option<u64>,
    pss_sampled_at: Option<f64>,
    pss_coverage: Option<PssCoverage>,
    pss_error: Option<String>,
    processes: Vec<ProcessInfo>,
    roles: HashMap<String, usize>,
    history: VecDeque<HistoryPoint>,
    cdp_mode: String,
    cdp_port: Option<u16>,
    cdp_state: String,
    cdp_reason: Option<String>,
    cdp_tabs: Vec<CdpTab>,
    cdp_checked_at: Option<f64>,
    ancestor_alive: HashMap<String, bool>,
    parent_state: String,
    parent_exited: bool,
    parent_exited_at: Option<f64>,
    launcher_exited: bool,
    launcher_exited_at: Option<f64>,
    driver_exited: bool,
    driver_exited_at: Option<f64>,
    watched: Option<(ProcessIdentity, String)>,
    watch_exited_at: Option<f64>,
    suspected: bool,
    suspected_at: Option<f64>,
    high_cpu: bool,
    high_cpu_since: Option<Instant>,
    high_cpu_at: Option<f64>,
    high_cpu_notified_at: Option<f64>,
    last_persist: Option<Instant>,
    stale: bool,
    terminating: bool,
    force_available: bool,
    action_notice: Option<String>,
    muted_until: Option<f64>,
}

pub struct PssStats {
    pub cpu_cores: Option<f64>,
    pub rss_sum: u64,
    pub process_count: usize,
    pub quality: String,
    pub pss_sum: Option<u64>,
}

pub struct Tracker {
    pub boot_id: String,
    sessions: HashMap<String, Session>,
    pub events: Vec<EventRow>,
    pub actions: Vec<ActionRow>,
    pub system_history: VecDeque<SystemPoint>,
    pub started_wall: f64,
    pub started_instant: Instant,
    pub last_ok_wall: Option<f64>,
    pub last_ok_instant: Option<Instant>,
    pub scan_failures: u64,
    pub last_error: Option<String>,
    pub online_cpus: usize,
    pending_metrics: Vec<MetricSample>,
    last_metric_flush: Instant,
}

fn session_id(identity: &ProcessIdentity) -> String {
    let digest = Sha256::digest(identity.key().as_bytes());
    format!("s-{}", &hex::encode(digest)[..14])
}

fn round(value: f64, places: i32) -> f64 {
    let factor = 10f64.powi(places);
    (value * factor).round() / factor
}

fn risk(session: &Session) -> String {
    if session.high_cpu || session.suspected {
        "warning".to_string()
    } else if session.launcher_exited || session.parent_exited || session.driver_exited {
        "info".to_string()
    } else {
        "none".to_string()
    }
}

impl Tracker {
    pub fn new(boot_id: String, online_cpus: usize) -> Self {
        Self {
            boot_id,
            sessions: HashMap::new(),
            events: Vec::new(),
            actions: Vec::new(),
            system_history: VecDeque::new(),
            started_wall: now_wall(),
            started_instant: Instant::now(),
            last_ok_wall: None,
            last_ok_instant: None,
            scan_failures: 0,
            last_error: None,
            online_cpus: online_cpus.max(1),
            pending_metrics: Vec::new(),
            last_metric_flush: Instant::now(),
        }
    }

    pub fn load_recent(&mut self, store: &Storage) {
        if let Ok(events) = store.recent_events(100) {
            self.events = events;
        }
        if let Ok(actions) = store.recent_actions(50) {
            self.actions = actions;
        }
    }

    pub fn active_ids(&self) -> HashSet<String> {
        self.sessions.keys().cloned().collect()
    }

    pub fn has_sessions(&self) -> bool {
        !self.sessions.is_empty()
    }

    pub fn termination_info(&self, id: &str) -> Option<(String, u32, u64, Option<u32>, bool)> {
        self.sessions.get(id).map(|session| {
            (
                session.boot_id.clone(),
                session.root_pid,
                session.root_start_ticks,
                session.uid,
                session.stale || session.terminating,
            )
        })
    }

    pub fn mark_terminating(&mut self, id: &str, mode: &str) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.terminating = true;
            session.force_available = false;
            session.action_notice = Some(format!(
                "{} requested; waiting for the scanner to confirm exit.",
                if mode == "kill" { "Force kill" } else { "Stop" }
            ));
        }
    }

    pub fn finish_termination(&mut self, id: &str, message: &str, still_alive: bool) {
        if let Some(session) = self.sessions.get_mut(id) {
            session.terminating = false;
            session.force_available = still_alive;
            session.action_notice = Some(redact_text(message, 220));
        }
    }

    pub fn mute_session(&mut self, id: &str, minutes: u64) -> bool {
        if let Some(session) = self.sessions.get_mut(id) {
            session.muted_until = Some(now_wall() + minutes.min(24 * 60) as f64 * 60.0);
            true
        } else {
            false
        }
    }

    pub fn update(
        &mut self,
        snapshot: ProcSnapshot,
        settings: &Settings,
        store: &mut Storage,
    ) -> Vec<Notify> {
        self.online_cpus = snapshot.online_cpus.max(1);
        let now_instant = snapshot.captured_at;
        let now = snapshot.captured_wall;
        if !snapshot.scan_ok {
            self.scan_failures += 1;
            self.last_error = snapshot.error.clone();
            for session in self.sessions.values_mut() {
                session.stale = true;
            }
            return Vec::new();
        }
        let recovered = self.scan_failures > 0;
        self.scan_failures = 0;
        self.last_error = None;
        self.last_ok_wall = Some(now);
        self.last_ok_instant = Some(now_instant);
        let mut pending = Vec::new();
        if recovered {
            pending.push(PendingEvent {
                session_id: None,
                kind: "scan_recovered".to_string(),
                severity: "info".to_string(),
                message: "Process scanning recovered after an error.".to_string(),
                evidence: json!({}),
                notify: None,
            });
        }

        let roots = browser_roots(&snapshot);
        let mut seen = HashSet::new();
        for root in roots.into_iter().take(ACTIVE_LIMIT) {
            let id = session_id(&root.identity());
            seen.insert(id.clone());
            if !self.sessions.contains_key(&id) {
                let stored = store.load_open_session(&id).ok().flatten();
                let session = Session::new(&snapshot, &root, settings, stored, self.started_wall);
                let notify =
                    if session.preexisting || !settings.notifications || settings.global_mute {
                        None
                    } else {
                        Some(Notify {
                            title: "Headless browser detected".to_string(),
                            body: format!(
                                "{} · PID {} · {}",
                                session.browser,
                                session.root_pid,
                                session.attribution.source_summary()
                            ),
                            session_id: Some(session.id.clone()),
                        })
                    };
                pending.push(PendingEvent {
                    session_id: Some(id.clone()),
                    kind: "session_started".to_string(),
                    severity: "info".to_string(),
                    message: format!(
                        "{} started · PID {} · {}",
                        session.browser,
                        session.root_pid,
                        session.attribution.source_summary()
                    ),
                    evidence: json!({
                        "pid": session.root_pid,
                        "start_ticks": session.root_start_ticks,
                        "discovery_mode": session.discovery_mode,
                        "source": session.attribution.source_summary(),
                    }),
                    notify,
                });
                self.sessions.insert(id.clone(), session);
            }
            let session = self.sessions.get_mut(&id).expect("inserted session");
            pending.extend(session.update(&snapshot, settings));
        }

        let ids = self.sessions.keys().cloned().collect::<Vec<_>>();
        for id in ids {
            if seen.contains(&id) {
                continue;
            }
            let unreadable = self
                .sessions
                .get(&id)
                .map(|session| snapshot.unreadable.contains(&session.root_pid))
                .unwrap_or(false);
            if unreadable {
                if let Some(session) = self.sessions.get_mut(&id) {
                    session.stale = true;
                }
                continue;
            }
            if let Some(session) = self.sessions.remove(&id) {
                let ended_at = now;
                let final_view = session.to_view(now, settings, "ended", self.online_cpus, true);
                if let Ok(mut value) = serde_json::to_value(&final_view) {
                    if let Some(object) = value.as_object_mut() {
                        object.insert("ended_at".to_string(), json!(ended_at));
                    }
                    if let Err(err) = store.end_session(&id, ended_at, &value) {
                        warn!("failed to persist ended session: {err}");
                    }
                }
                pending.push(PendingEvent {
                    session_id: Some(id.clone()),
                    kind: "session_ended".to_string(),
                    severity: "info".to_string(),
                    message: format!(
                        "{} exited · PID {} · ran {}",
                        session.browser,
                        session.root_pid,
                        format_duration((ended_at - session.launched_at) as i64)
                    ),
                    evidence: json!({
                        "pid": session.root_pid,
                        "start_ticks": session.root_start_ticks,
                        "ended_at": ended_at,
                    }),
                    notify: None,
                });
            }
        }

        let mut notices = Vec::new();
        for mut item in pending {
            if let Some(notify) = item.notify.take() {
                notices.push(notify);
            }
            self.emit(store, item);
        }

        for session in self.sessions.values_mut() {
            let persist_due = session.last_persist.map_or(true, |last| {
                now_instant.duration_since(last) >= Duration::from_secs(5)
            });
            if persist_due {
                if let Err(err) = store.upsert_session(&session.record(now)) {
                    warn!("failed to persist session: {err}");
                }
                session.last_persist = Some(now_instant);
            }
            self.pending_metrics.push(MetricSample {
                session_id: session.id.clone(),
                ts: now,
                cpu_cores: session.cpu.map(|value| value.cores),
                rss_sum: session.rss_sum,
                pss_sum: session.pss_sum,
                process_count: session.processes.len(),
                quality: session.cpu_quality.clone(),
            });
        }
        if self.pending_metrics.len() >= 5
            || self.last_metric_flush.elapsed() >= Duration::from_secs(5)
        {
            if let Err(err) = store.insert_metrics_batch(&self.pending_metrics) {
                warn!("failed to persist metric batch: {err}");
            }
            self.pending_metrics.clear();
            self.last_metric_flush = Instant::now();
        }

        let cores = self
            .sessions
            .values()
            .map(|session| session.cpu.map(|value| value.cores).unwrap_or(0.0))
            .sum::<f64>();
        let rss = self
            .sessions
            .values()
            .map(|session| session.rss_sum)
            .sum::<u64>();
        let pss_values = self
            .sessions
            .values()
            .filter_map(|session| session.pss_sum)
            .collect::<Vec<_>>();
        self.system_history.push_back(SystemPoint {
            t: now,
            cores: round(cores, 6),
            machine_pct: round(machine_pct(cores, self.online_cpus), 4),
            rss_sum: rss,
            pss_sum: (!pss_values.is_empty()).then_some(pss_values.iter().sum()),
        });
        while self.system_history.len() > settings.history_max {
            self.system_history.pop_front();
        }

        notices
    }

    fn emit(&mut self, store: &mut Storage, item: PendingEvent) {
        let evidence = item.evidence;
        match store.insert_event(
            item.session_id.as_deref(),
            now_wall(),
            &item.kind,
            &item.severity,
            &item.message,
            &evidence,
        ) {
            Ok(event) => {
                self.events.insert(0, event);
                self.events.truncate(200);
            }
            Err(err) => {
                warn!("failed to persist event: {err}");
                self.events.insert(
                    0,
                    EventRow {
                        id: 0,
                        t: now_wall(),
                        at: iso_timestamp(Some(now_wall())),
                        session_id: item.session_id,
                        kind: item.kind,
                        severity: item.severity,
                        message: item.message,
                        evidence,
                        acknowledged: false,
                        muted_until: None,
                    },
                );
                self.events.truncate(200);
            }
        }
    }

    pub fn health(&self, now_instant: Instant) -> HealthView {
        let status = if self.last_ok_instant.is_none() {
            "starting"
        } else if self.scan_failures > 0 {
            "degraded"
        } else {
            "ok"
        };
        HealthView {
            platform: std::env::consts::OS,
            pss_supported: cfg!(target_os = "linux"),
            graceful_stop_supported: cfg!(target_os = "linux"),
            status: status.to_string(),
            last_scan_age_s: self
                .last_ok_instant
                .map(|value| round(now_instant.duration_since(value).as_secs_f64(), 1)),
            last_scan_at: iso_timestamp(self.last_ok_wall),
            scan_failures: self.scan_failures,
            last_error: self.last_error.clone(),
            stale_sessions: self
                .sessions
                .values()
                .filter(|session| session.stale)
                .count(),
            online_cpus: self.online_cpus,
            monitor_pid: std::process::id(),
            active_sessions: self.sessions.len(),
        }
    }

    pub fn summary(&self, settings: &Settings, now: f64) -> SummaryView {
        let sessions = self.sessions.values().collect::<Vec<_>>();
        let cores = sessions
            .iter()
            .map(|session| session.cpu.map(|value| value.cores).unwrap_or(0.0))
            .sum::<f64>();
        let rss = sessions.iter().map(|session| session.rss_sum).sum::<u64>();
        let pss_values = sessions
            .iter()
            .filter_map(|session| session.pss_sum)
            .collect::<Vec<_>>();
        let pss_sum = (!pss_values.is_empty()).then_some(pss_values.iter().sum());
        let pss_sampled = sessions
            .iter()
            .filter_map(|session| session.pss_sampled_at)
            .max_by(f64::total_cmp);
        let _ = settings;
        SummaryView {
            active_sessions: sessions.len(),
            process_count: sessions.iter().map(|session| session.processes.len()).sum(),
            cpu_cores: round(cores, 3),
            cpu_single_core_pct: round(cores * 100.0, 1),
            cpu_machine_pct: round(machine_pct(cores, self.online_cpus), 2),
            online_cpus: self.online_cpus,
            calculating_sessions: sessions
                .iter()
                .filter(|session| session.cpu.is_none())
                .count(),
            rss_sum: rss,
            pss_sum,
            pss_age_s: pss_sampled.map(|value| round((now - value).max(0.0), 1)),
            parent_exited: sessions
                .iter()
                .filter(|session| session.parent_exited)
                .count(),
            suspected_leftovers: sessions.iter().filter(|session| session.suspected).count(),
            high_cpu: sessions.iter().filter(|session| session.high_cpu).count(),
            unknown_origin: sessions
                .iter()
                .filter(|session| session.attribution.launcher == "Unknown")
                .count(),
        }
    }

    pub fn session_views(&self, settings: &Settings, now: f64) -> Vec<SessionView> {
        let mut views = self
            .sessions
            .values()
            .map(|session| {
                session.to_view(
                    now,
                    settings,
                    "active",
                    self.online_cpus,
                    settings.allow_screenshots,
                )
            })
            .collect::<Vec<_>>();
        views.sort_by(|left, right| {
            right
                .cpu
                .cores
                .unwrap_or(0.0)
                .total_cmp(&left.cpu.cores.unwrap_or(0.0))
        });
        views
    }

    pub fn pss_targets(&self) -> Vec<(String, Vec<(u32, u64)>)> {
        self.sessions
            .values()
            .map(|session| {
                (
                    session.id.clone(),
                    session
                        .processes
                        .iter()
                        .map(|proc| (proc.pid, proc.start_ticks))
                        .collect(),
                )
            })
            .collect()
    }

    pub fn apply_pss(
        &mut self,
        session_id: &str,
        values: Vec<(u32, u64)>,
        errors: Vec<String>,
        sampled_at: f64,
    ) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            let total = session.processes.len();
            let sampled = values.len();
            session.pss_sum =
                (!values.is_empty()).then_some(values.iter().map(|(_, value)| value).sum());
            session.pss_coverage = Some(PssCoverage { sampled, total });
            session.pss_sampled_at = Some(sampled_at);
            session.pss_error = if values.is_empty() {
                errors.first().cloned()
            } else {
                None
            };
        }
    }

    pub fn pss_stats(&self, session_id: &str) -> Option<PssStats> {
        self.sessions.get(session_id).map(|session| PssStats {
            cpu_cores: session.cpu.map(|value| value.cores),
            rss_sum: session.rss_sum,
            process_count: session.processes.len(),
            quality: session.cpu_quality.clone(),
            pss_sum: session.pss_sum,
        })
    }

    pub fn tabs_json(&self, session_id: &str) -> Option<Value> {
        self.sessions.get(session_id).map(|session| {
            json!({
                "session_id": session.id,
                "mode": session.cdp_mode,
                "state": session.cdp_state,
                "port": session.cdp_port,
                "reason": session.cdp_reason,
                "tabs": session.cdp_tabs,
            })
        })
    }

    pub fn cdp_targets(&self) -> Vec<(String, u16)> {
        self.sessions
            .values()
            .filter_map(|session| session.cdp_port.map(|port| (session.id.clone(), port)))
            .collect()
    }

    pub fn apply_cdp(&mut self, session_id: &str, port: u16, probe: CdpProbe, checked_at: f64) {
        if let Some(session) = self.sessions.get_mut(session_id) {
            if session.cdp_port != Some(port) {
                return;
            }
            session.cdp_state = probe.state;
            session.cdp_reason = probe.reason;
            session.cdp_tabs = probe.tabs;
            session.cdp_checked_at = Some(checked_at);
        }
    }

    pub fn screenshot_tab(&self, session_id: &str, tab_id: &str) -> Option<(u16, String)> {
        let session = self.sessions.get(session_id)?;
        if session.cdp_mode != "tcp" || session.cdp_state != "available" {
            return None;
        }
        let port = session.cdp_port?;
        session
            .cdp_tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .map(|tab| (port, tab.ws_url.clone()))
    }

    pub fn session_json(&self, id: &str, settings: &Settings, now: f64) -> Option<Value> {
        self.sessions.get(id).and_then(|session| {
            serde_json::to_value(session.to_view(
                now,
                settings,
                "active",
                self.online_cpus,
                settings.allow_screenshots,
            ))
            .ok()
        })
    }
}

impl Session {
    fn new(
        snapshot: &ProcSnapshot,
        root: &ProcessInfo,
        settings: &Settings,
        stored: Option<crate::storage::HistorySession>,
        monitor_start_wall: f64,
    ) -> Self {
        let id = session_id(&root.identity());
        let observed_launched_at = crate::procfs::launched_at(snapshot, root.start_ticks);
        let (first_seen_at, launched_at, discovery_mode, attribution, preexisting) =
            if let Some(stored) = stored {
                let attribution = serde_json::from_value::<Attribution>(stored.source.clone())
                    .unwrap_or_else(|_| build_attribution(snapshot, root, None));
                let preexisting = stored.discovery_mode == "preexisting_at_start";
                (
                    stored.first_seen_at,
                    stored.launched_at,
                    stored.discovery_mode,
                    attribution,
                    preexisting,
                )
            } else {
                let preexisting = observed_launched_at < monitor_start_wall - 0.5;
                let mode = if preexisting {
                    "preexisting_at_start"
                } else {
                    "observed_live"
                };
                (
                    snapshot.captured_wall,
                    observed_launched_at,
                    mode.to_string(),
                    build_attribution(snapshot, root, None),
                    preexisting,
                )
            };
        let watched = attribution.watched_identity();
        let endpoint = cdp_endpoint(root);
        let mut session = Self {
            id,
            root_identity: root.identity(),
            boot_id: root.boot_id.clone(),
            root_pid: root.pid,
            root_start_ticks: root.start_ticks,
            browser: crate::browser::browser_family(root),
            executable: root.executable_basename(),
            command: own_command(root),
            cgroup: root.cgroup.clone(),
            uid: root.uid,
            discovery_mode,
            preexisting,
            first_seen_at,
            launched_at,
            attribution,
            online_cpus: snapshot.online_cpus,
            last_scan_instant: None,
            prev_ticks: HashMap::new(),
            cpu: None,
            peak_cpu: None,
            cpu_quality: "first_sample".to_string(),
            rss_sum: 0,
            pss_sum: None,
            pss_sampled_at: None,
            pss_coverage: None,
            pss_error: None,
            processes: Vec::new(),
            roles: HashMap::new(),
            history: VecDeque::new(),
            cdp_mode: "none".to_string(),
            cdp_port: None,
            cdp_state: "no_endpoint".to_string(),
            cdp_reason: None,
            cdp_tabs: Vec::new(),
            cdp_checked_at: None,
            ancestor_alive: HashMap::new(),
            parent_state: "not_observable".to_string(),
            parent_exited: false,
            parent_exited_at: None,
            launcher_exited: false,
            launcher_exited_at: None,
            driver_exited: false,
            driver_exited_at: None,
            watched,
            watch_exited_at: None,
            suspected: false,
            suspected_at: None,
            high_cpu: false,
            high_cpu_since: None,
            high_cpu_at: None,
            high_cpu_notified_at: None,
            last_persist: None,
            stale: false,
            terminating: false,
            force_available: false,
            action_notice: None,
            muted_until: None,
        };
        session.set_cdp(endpoint);
        let _ = settings;
        session
    }

    fn set_cdp(&mut self, endpoint: CdpEndpoint) {
        if endpoint.mode == "tcp" {
            if self.cdp_port != endpoint.port {
                self.cdp_port = endpoint.port;
                self.cdp_mode = "tcp".to_string();
                self.cdp_state = "checking".to_string();
                self.cdp_reason =
                    Some("Probing the browser's local debugging endpoint.".to_string());
                self.cdp_tabs.clear();
                self.cdp_checked_at = None;
            }
        } else {
            self.cdp_port = None;
            self.cdp_mode = endpoint.mode;
            self.cdp_state = if self.cdp_mode == "pipe" {
                "pipe_only".to_string()
            } else {
                "no_endpoint".to_string()
            };
            self.cdp_reason = endpoint.reason;
            self.cdp_tabs.clear();
            self.cdp_checked_at = None;
        }
    }

    fn update(&mut self, snapshot: &ProcSnapshot, settings: &Settings) -> Vec<PendingEvent> {
        self.stale = false;
        self.online_cpus = snapshot.online_cpus.max(1);
        let now = snapshot.captured_wall;
        let processes = descendants(snapshot, self.root_pid, self.uid, 5000);
        let elapsed = self
            .last_scan_instant
            .map(|previous| snapshot.captured_at.duration_since(previous).as_secs_f64())
            .filter(|value| *value > 0.0);
        let current_ticks = processes
            .iter()
            .map(|proc| (proc.identity().key(), proc.cpu_ticks))
            .collect::<HashMap<_, _>>();
        let mut delta_ticks = 0i64;
        for (key, ticks) in &current_ticks {
            if let Some(previous) = self.prev_ticks.get(key) {
                delta_ticks += ticks.saturating_sub(*previous) as i64;
            }
        }
        if let Some(elapsed) = elapsed {
            if !self.prev_ticks.is_empty() {
                let value = CpuValue::from_delta(delta_ticks, elapsed, self.online_cpus);
                self.cpu = Some(value);
                self.peak_cpu = Some(
                    self.peak_cpu
                        .map_or(value.cores, |peak| peak.max(value.cores)),
                );
                self.cpu_quality = if elapsed > settings.interval_s * 5.0 {
                    "gap".to_string()
                } else {
                    "ok".to_string()
                };
                self.history.push_back(HistoryPoint {
                    t: now,
                    cores: value.cores,
                });
                while self.history.len() > settings.history_max {
                    self.history.pop_front();
                }
            } else {
                self.cpu = None;
                self.cpu_quality = "first_sample".to_string();
            }
        }
        self.prev_ticks = current_ticks;
        self.last_scan_instant = Some(snapshot.captured_at);

        self.processes = processes;
        self.rss_sum = self.processes.iter().map(|proc| proc.rss_bytes).sum();
        self.roles.clear();
        for proc in &self.processes {
            let role = process_role(proc).unwrap_or_else(|| "browser".to_string());
            *self.roles.entry(role).or_insert(0) += 1;
        }
        if let Some(root) = snapshot.processes.get(&self.root_pid) {
            self.set_cdp(cdp_endpoint(root));
        }

        let mut pending = Vec::new();
        let now_instant = snapshot.captured_at;
        for ancestor in &self.attribution.ancestors {
            let state = identity_alive(snapshot, &ancestor.identity);
            self.ancestor_alive
                .insert(ancestor.identity.key(), state == "alive");
        }
        if let Some(parent) = &self.attribution.direct_parent {
            let state = identity_alive(snapshot, parent);
            self.parent_state = state.to_string();
            if state != "alive" && !self.parent_exited {
                self.parent_exited = true;
                self.parent_exited_at = Some(now);
                pending.push(PendingEvent {
                    session_id: Some(self.id.clone()),
                    kind: "parent_exited".to_string(),
                    severity: "info".to_string(),
                    message: format!(
                        "{} · parent/proxy exited while the browser remains.",
                        self.browser
                    ),
                    evidence: json!({"parent_pid": parent.pid, "parent_state": state}),
                    notify: None,
                });
            }
        }
        if let (Some(identity), true) = (
            &self.attribution.launcher_identity,
            self.attribution.launcher_confidence == "observed",
        ) {
            if identity_alive(snapshot, identity) != "alive" && !self.launcher_exited {
                self.launcher_exited = true;
                self.launcher_exited_at = Some(now);
                pending.push(PendingEvent {
                    session_id: Some(self.id.clone()),
                    kind: "launcher_exited".to_string(),
                    severity: "warning".to_string(),
                    message: format!(
                        "{} exited while {} PID {} is still running.",
                        self.attribution.launcher, self.browser, self.root_pid
                    ),
                    evidence: json!({"launcher": self.attribution.launcher, "pid": identity.pid}),
                    notify: None,
                });
            }
        }
        if let (Some(identity), true) = (
            &self.attribution.driver_identity,
            self.attribution.driver_confidence == "observed",
        ) {
            if identity_alive(snapshot, identity) != "alive" && !self.driver_exited {
                self.driver_exited = true;
                self.driver_exited_at = Some(now);
                pending.push(PendingEvent {
                    session_id: Some(self.id.clone()),
                    kind: "driver_exited".to_string(),
                    severity: "info".to_string(),
                    message: format!(
                        "{} driver exited while {} PID {} is still running.",
                        self.attribution.driver, self.browser, self.root_pid
                    ),
                    evidence: json!({"driver": self.attribution.driver, "pid": identity.pid}),
                    notify: None,
                });
            }
        }
        if self.watch_exited_at.is_none() {
            if let Some((identity, label)) = &self.watched {
                if identity_alive(snapshot, identity) != "alive" {
                    self.watch_exited_at = Some(now);
                    pending.push(PendingEvent {
                        session_id: Some(self.id.clone()),
                        kind: "watch_exited".to_string(),
                        severity: "info".to_string(),
                        message: format!(
                            "The observed {} process exited; tracking the browser for suspected leftover status.",
                            label
                        ),
                        evidence: json!({"watched_pid": identity.pid, "label": label}),
                        notify: None,
                    });
                }
            }
        }
        if !self.suspected {
            if let Some(exited_at) = self.watch_exited_at {
                if now - exited_at >= settings.leftover_after_s {
                    self.suspected = true;
                    self.suspected_at = Some(now);
                    pending.push(PendingEvent {
                        session_id: Some(self.id.clone()),
                        kind: "suspected_leftover".to_string(),
                        severity: "warning".to_string(),
                        message: format!(
                            "{} PID {} is still running {}s after its observed parent/launcher exited.",
                            self.browser, self.root_pid, settings.leftover_after_s as i64
                        ),
                        evidence: json!({"watch_exited_at": exited_at, "delay_s": settings.leftover_after_s}),
                        notify: (!settings.global_mute && settings.notifications && !self.muted(now)).then(|| Notify {
                            title: "Suspected leftover browser".to_string(),
                            body: format!("{} · PID {} · source exited", self.browser, self.root_pid),
                            session_id: Some(self.id.clone()),
                        }),
                    });
                }
            }
        }
        self.update_high_cpu(now, now_instant, settings, &mut pending);
        pending
    }

    fn update_high_cpu(
        &mut self,
        now: f64,
        now_instant: Instant,
        settings: &Settings,
        pending: &mut Vec<PendingEvent>,
    ) {
        let Some(cpu) = self.cpu else {
            return;
        };
        if cpu.cores >= settings.cpu_alert_cores {
            if self.high_cpu_since.is_none() {
                self.high_cpu_since = Some(now_instant);
            }
            let sustained = self
                .high_cpu_since
                .map(|value| now_instant.duration_since(value).as_secs_f64())
                .unwrap_or(0.0);
            if !self.high_cpu && sustained >= settings.cpu_alert_duration_s {
                self.high_cpu = true;
                self.high_cpu_at = Some(now);
                let cooldown_active = self
                    .high_cpu_notified_at
                    .is_some_and(|notified| now - notified < 600.0);
                let notify = (!cooldown_active
                    && !settings.global_mute
                    && settings.notifications
                    && !self.muted(now))
                .then(|| Notify {
                    title: "Headless browser high CPU".to_string(),
                    body: format!(
                        "{} · PID {} · {:.2} cores for {}s",
                        self.browser,
                        self.root_pid,
                        cpu.cores,
                        settings.cpu_alert_duration_s as i64
                    ),
                    session_id: Some(self.id.clone()),
                });
                if notify.is_some() {
                    self.high_cpu_notified_at = Some(now);
                }
                pending.push(PendingEvent {
                    session_id: Some(self.id.clone()),
                    kind: "high_cpu".to_string(),
                    severity: "warning".to_string(),
                    message: format!(
                        "{} PID {} sustained {:.2} cores for {}s.",
                        self.browser,
                        self.root_pid,
                        cpu.cores,
                        settings.cpu_alert_duration_s as i64
                    ),
                    evidence: json!({
                        "cores": round(cpu.cores, 3),
                        "machine_pct": round(cpu.machine_pct, 2),
                        "threshold_cores": settings.cpu_alert_cores,
                        "duration_s": settings.cpu_alert_duration_s,
                    }),
                    notify,
                });
            }
        } else {
            if self.high_cpu_since.is_some()
                && cpu.cores < settings.cpu_alert_cores * settings.cpu_clear_ratio
            {
                self.high_cpu_since = None;
                if self.high_cpu {
                    self.high_cpu = false;
                    self.high_cpu_at = None;
                    pending.push(PendingEvent {
                        session_id: Some(self.id.clone()),
                        kind: "high_cpu_cleared".to_string(),
                        severity: "info".to_string(),
                        message: format!(
                            "{} PID {} is back below the CPU alert threshold.",
                            self.browser, self.root_pid
                        ),
                        evidence: json!({"cores": round(cpu.cores, 3)}),
                        notify: None,
                    });
                }
            }
        }
    }

    fn muted(&self, now: f64) -> bool {
        self.muted_until.is_some_and(|until| until > now)
    }

    fn source_view(&self) -> SourceView {
        SourceView {
            summary: self.attribution.source_summary(),
            confidence: self.attribution.origin_confidence().to_string(),
            launcher: self.attribution.launcher.clone(),
            launcher_confidence: self.attribution.launcher_confidence.clone(),
            launcher_pid: self.attribution.launcher_pid,
            launcher_name: self.attribution.launcher_name.clone(),
            driver: self.attribution.driver.clone(),
            driver_confidence: self.attribution.driver_confidence.clone(),
            driver_pid: self.attribution.driver_pid,
            driver_name: self.attribution.driver_name.clone(),
        }
    }

    fn parent_view(&self) -> ParentView {
        ParentView {
            pid: self.attribution.direct_parent_pid,
            name: self.attribution.direct_parent_name.clone(),
            observable: self.attribution.parent_observable,
            state: self.parent_state.clone(),
            exited: self.parent_exited,
            exited_at: self.parent_exited_at,
            note: self.parent_exited.then(|| {
                "Direct parent exited; this is an observation, not a determination that the browser is unwanted.".to_string()
            }),
        }
    }

    fn to_view(
        &self,
        now: f64,
        _settings: &Settings,
        lifecycle: &str,
        online_cpus: usize,
        screenshots: bool,
    ) -> SessionView {
        let cpu = self
            .cpu
            .map(|value| CpuView {
                cores: Some(round(value.cores, 3)),
                single_core_pct: Some(round(value.single_core_pct, 1)),
                machine_pct: Some(round(machine_pct(value.cores, online_cpus), 2)),
                peak_cores: self.peak_cpu.map(|peak| round(peak, 3)),
                quality: if self.stale {
                    "stale".to_string()
                } else {
                    self.cpu_quality.clone()
                },
                online_cpus,
            })
            .unwrap_or(CpuView {
                cores: None,
                single_core_pct: None,
                machine_pct: None,
                peak_cores: self.peak_cpu.map(|peak| round(peak, 3)),
                quality: if self.stale {
                    "stale".to_string()
                } else {
                    self.cpu_quality.clone()
                },
                online_cpus,
            });
        let pss_quality = if cfg!(windows) {
            "unsupported"
        } else if self.pss_sum.is_some() {
            if self
                .pss_coverage
                .as_ref()
                .is_some_and(|coverage| coverage.sampled < coverage.total)
            {
                "partial"
            } else {
                "full"
            }
        } else if self.pss_error.is_some() {
            "unavailable"
        } else {
            "not_sampled"
        };
        let memory = MemoryView {
            rss_sum: self.rss_sum,
            rss_quality: if cfg!(windows) {
                "working_set_upper_bound"
            } else {
                "upper_bound_estimate"
            }
            .to_string(),
            pss_sum: self.pss_sum,
            pss_quality: pss_quality.to_string(),
            pss_age_s: self
                .pss_sampled_at
                .map(|value| round((now - value).max(0.0), 1)),
            pss_coverage: self.pss_coverage.clone(),
            pss_error: self.pss_error.clone(),
        };
        let status = StatusView {
            lifecycle: lifecycle.to_string(),
            stale: self.stale,
            parent_exited: self.parent_exited,
            parent_state: self.parent_state.clone(),
            launcher_exited: self.launcher_exited,
            launcher_exited_at: self.launcher_exited_at,
            driver_exited: self.driver_exited,
            driver_exited_at: self.driver_exited_at,
            watch_exited_at: self.watch_exited_at,
            suspected_leftover: self.suspected,
            suspected_at: self.suspected_at,
            high_cpu: self.high_cpu,
            high_cpu_at: self.high_cpu_at,
            terminating: self.terminating,
            risk: risk(self),
            muted_until: self.muted_until.filter(|until| *until > now),
        };
        let ancestry = self
            .attribution
            .ancestors
            .iter()
            .map(|entry| AncestorView {
                pid: entry.pid,
                ppid: entry.ppid,
                name: entry.name.clone(),
                command: entry.command.clone(),
                role: entry.role.clone(),
                start_ticks: entry.identity.start_ticks,
                alive: self
                    .ancestor_alive
                    .get(&entry.identity.key())
                    .copied()
                    .unwrap_or(false),
            })
            .collect();
        let processes = self
            .processes
            .iter()
            .map(|proc| ProcessView {
                pid: proc.pid,
                ppid: proc.ppid,
                name: proc.name.clone(),
                role: process_role(proc).unwrap_or_else(|| "browser".to_string()),
                command: redact_argv(&proc.argv, 180),
                uid: proc.uid,
                rss_bytes: proc.rss_bytes,
            })
            .collect();
        let history = self
            .history
            .iter()
            .rev()
            .take(120)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        SessionView {
            id: self.id.clone(),
            pid: self.root_pid,
            start_ticks: self.root_start_ticks,
            identity: self.root_identity.clone(),
            browser: self.browser.clone(),
            executable: self.executable.clone(),
            command: redact_argv(
                &self
                    .command
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
                420,
            ),
            cgroup: self.cgroup.as_deref().map(redact_path),
            uid: self.uid,
            discovery_mode: self.discovery_mode.clone(),
            preexisting: self.preexisting,
            first_seen_at: self.first_seen_at,
            launched_at: self.launched_at,
            ended_at: None,
            lifetime_s: (now - self.launched_at).max(0.0) as i64,
            source: self.source_view(),
            ancestry,
            parent: self.parent_view(),
            status,
            cpu,
            memory,
            process_count: self.processes.len(),
            processes,
            roles: self.roles.clone(),
            history,
            cdp: CdpView {
                mode: self.cdp_mode.clone(),
                state: self.cdp_state.clone(),
                port: self.cdp_port,
                reason: self.cdp_reason.clone(),
                tabs: self.cdp_tabs.clone(),
                screenshot_available: screenshots,
            },
            can_terminate: !self.stale && !self.terminating,
            force_available: self.force_available,
            action_notice: self.action_notice.clone(),
        }
    }

    fn record(&self, now: f64) -> SessionRecord {
        SessionRecord {
            id: self.id.clone(),
            boot_id: self.boot_id.clone(),
            root_pid: self.root_pid,
            root_start_ticks: self.root_start_ticks,
            browser: self.browser.clone(),
            executable: self.executable.clone(),
            command: redact_argv(
                &self
                    .command
                    .split_whitespace()
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
                420,
            ),
            cgroup: self.cgroup.as_deref().map(redact_path),
            uid: self.uid,
            launched_at: self.launched_at,
            first_seen_at: self.first_seen_at,
            last_seen_at: now,
            discovery_mode: self.discovery_mode.clone(),
            source: serde_json::to_value(&self.attribution).unwrap_or_else(|_| json!({})),
            ancestry: serde_json::to_value(&self.attribution.ancestors)
                .unwrap_or_else(|_| json!([])),
            parent: serde_json::to_value(self.parent_view()).unwrap_or_else(|_| json!({})),
            status: json!({
                "lifecycle": "active",
                "stale": self.stale,
                "parent_exited": self.parent_exited,
                "launcher_exited": self.launcher_exited,
                "driver_exited": self.driver_exited,
                "suspected_leftover": self.suspected,
                "high_cpu": self.high_cpu,
                "risk": risk(self),
            }),
            peak_cpu: self.peak_cpu.map(|value| round(value, 3)),
            latest_rss: self.rss_sum,
            latest_pss: self.pss_sum,
            latest_process_count: self.processes.len(),
        }
    }
}

pub fn overview(tracker: &Tracker, settings: &Settings, store: &Storage, revision: u64) -> Value {
    let now = now_wall();
    let recent_ended = store.list_ended_summaries(10).unwrap_or_default();
    let mut value = json!({
        "version": VERSION,
        "generated_at": now,
        "uptime_s": (now - tracker.started_wall).max(0.0) as i64,
        "revision": revision,
        "health": tracker.health(Instant::now()),
        "summary": tracker.summary(settings, now),
        "sessions": tracker.session_views(settings, now),
        "recent_ended": recent_ended,
        "events": tracker.events,
        "actions": tracker.actions,
        "system_history": tracker.system_history,
        "settings": settings,
        "database_size": store.database_size(),
    });
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "started_at".to_string(),
            json!(iso_timestamp(Some(tracker.started_wall))),
        );
    }
    value
}

pub fn history_biography(session: &crate::storage::HistorySession) -> Value {
    if let Some(final_view) = &session.final_view {
        return final_view.clone();
    }
    json!({
        "id": session.id,
        "pid": session.root_pid,
        "start_ticks": session.root_start_ticks,
        "browser": session.browser,
        "executable": session.executable,
        "command": session.command,
        "cgroup": session.cgroup,
        "uid": session.uid,
        "discovery_mode": session.discovery_mode,
        "first_seen_at": session.first_seen_at,
        "launched_at": session.launched_at,
        "ended_at": session.ended_at,
        "lifetime_s": session.ended_at.map(|end| (end - session.launched_at).max(0.0) as i64),
        "source": session.source,
        "ancestry": session.ancestry,
        "parent": session.parent,
        "status": session.status,
        "peak_cpu": session.peak_cpu,
        "memory": {
            "rss_sum": session.latest_rss,
            "pss_sum": session.latest_pss,
        },
        "process_count": session.latest_process_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProcSnapshot, ProcessInfo};
    use crate::procfs::clk_tck;
    use crate::storage::Storage;
    use std::collections::{HashMap, HashSet};
    use std::time::Duration;

    fn process(
        pid: u32,
        ppid: u32,
        exe: &str,
        argv: &[&str],
        start_ticks: u64,
        cpu_ticks: u64,
    ) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            start_ticks,
            cpu_ticks,
            rss_bytes: 1024,
            state: "S".to_string(),
            name: exe.rsplit('/').next().unwrap_or(exe).to_string(),
            exe: exe.to_string(),
            argv: argv.iter().map(|value| value.to_string()).collect(),
            uid: Some(1000),
            cgroup: Some("/user.slice/session.scope".to_string()),
            boot_id: "boot".to_string(),
        }
    }

    fn snapshot(
        processes: Vec<ProcessInfo>,
        base_at: Instant,
        elapsed_s: f64,
        base_wall: f64,
    ) -> ProcSnapshot {
        let pids = processes
            .iter()
            .map(|proc| proc.pid)
            .collect::<HashSet<_>>();
        ProcSnapshot {
            boot_id: "boot".to_string(),
            uptime_s: 100.0 + elapsed_s,
            processes: processes
                .into_iter()
                .map(|proc| (proc.pid, proc))
                .collect::<HashMap<_, _>>(),
            captured_at: base_at + Duration::from_secs_f64(elapsed_s),
            captured_wall: base_wall + elapsed_s,
            pids_seen: pids,
            unreadable: HashSet::new(),
            zombie_pids: HashSet::new(),
            scan_ok: true,
            error: None,
            online_cpus: 8,
        }
    }

    fn chrome(pid: u32, ppid: u32, start_ticks: u64, cpu_ticks: u64) -> ProcessInfo {
        process(
            pid,
            ppid,
            "/opt/google/chrome/chrome",
            &["--headless=new", "--remote-debugging-pipe"],
            start_ticks,
            cpu_ticks,
        )
    }

    #[test]
    fn preexisting_chrome_has_unknown_origin() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let root = chrome(500, 1, (90.0 * clk_tck()) as u64, 0);
        let settings = Settings::default();
        tracker.update(
            snapshot(vec![root], base_at, 0.0, base_wall),
            &settings,
            &mut store,
        );
        let views = tracker.session_views(&settings, base_wall);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].discovery_mode, "preexisting_at_start");
        assert!(views[0].preexisting);
        assert_eq!(views[0].source.launcher, "Unknown");
        assert_eq!(views[0].source.confidence, "unknown");
    }

    #[test]
    fn tracks_launcher_exit_and_suspected_leftover_evidence() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let root = chrome(400, 300, (99.0 * clk_tck()) as u64, 0);
        let driver = process(
            300,
            200,
            "/usr/bin/node",
            &["node", "/opt/playwright-mcp/index.js"],
            90,
            0,
        );
        let launcher = process(200, 1, "/usr/bin/opencode", &["opencode"], 80, 0);
        let mut settings = Settings::default();
        settings.leftover_after_s = 10.0;
        settings.notifications = true;

        let notices = tracker.update(
            snapshot(
                vec![root.clone(), driver.clone(), launcher],
                base_at,
                0.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        assert!(notices
            .iter()
            .any(|notice| notice.title == "Headless browser detected"));
        let views = tracker.session_views(&settings, base_wall);
        assert_eq!(views[0].source.summary, "OpenCode / Playwright");

        let exit_notices = tracker.update(
            snapshot(vec![root.clone(), driver.clone()], base_at, 2.0, base_wall),
            &settings,
            &mut store,
        );
        assert!(!exit_notices
            .iter()
            .any(|notice| notice.title.contains("launcher exited")));
        assert!(!exit_notices
            .iter()
            .any(|notice| notice.title.contains("parent exited")));
        tracker.update(
            snapshot(vec![root.clone(), driver], base_at, 20.0, base_wall),
            &settings,
            &mut store,
        );
        let views = tracker.session_views(&settings, base_wall + 20.0);
        assert!(views[0].status.launcher_exited);
        assert!(views[0].status.suspected_leftover);
        assert_eq!(views[0].source.summary, "OpenCode / Playwright");
        assert!(views[0]
            .ancestry
            .iter()
            .any(|entry| entry.name == "opencode" && !entry.alive));
        assert!(tracker
            .events
            .iter()
            .any(|event| event.kind == "suspected_leftover"));

        tracker.update(
            snapshot(vec![root], base_at, 30.0, base_wall),
            &settings,
            &mut store,
        );
        let views = tracker.session_views(&settings, base_wall + 30.0);
        assert!(views[0].status.parent_exited);
        assert!(views[0].status.driver_exited);
        assert!(tracker
            .events
            .iter()
            .any(|event| event.kind == "parent_exited"));
    }

    #[test]
    fn ends_session_when_root_disappears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let settings = Settings::default();
        tracker.update(
            snapshot(
                vec![chrome(900, 1, (99.0 * clk_tck()) as u64, 0)],
                base_at,
                0.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        let session_id = tracker.session_views(&settings, base_wall)[0].id.clone();
        tracker.update(
            snapshot(Vec::new(), base_at, 2.0, base_wall),
            &settings,
            &mut store,
        );
        assert!(tracker.sessions.is_empty());
        let ended = store.list_ended_sessions(10).expect("ended sessions");
        assert!(ended
            .iter()
            .any(|row| row.id == session_id && row.ended_at.is_some()));
        assert!(tracker
            .events
            .iter()
            .any(|event| event.kind == "session_ended"));
    }

    #[test]
    fn handles_hundred_process_tree_quickly() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let settings = Settings::default();
        let root = chrome(1000, 1, (99.0 * clk_tck()) as u64, 0);
        let mut processes = vec![root];
        for pid in 1001..1100 {
            processes.push(process(
                pid,
                1000,
                "/opt/google/chrome/chrome",
                &["--type=renderer", "--headless=new"],
                (99.0 * clk_tck()) as u64,
                0,
            ));
        }
        let started = Instant::now();
        tracker.update(
            snapshot(processes, base_at, 0.0, base_wall),
            &settings,
            &mut store,
        );
        let elapsed = started.elapsed();
        let views = tracker.session_views(&settings, base_wall);
        assert_eq!(views.len(), 1);
        assert_eq!(views[0].process_count, 100);
        assert!(
            elapsed < Duration::from_secs(2),
            "100-process update took {elapsed:?}"
        );
    }

    #[test]
    fn alerts_once_for_sustained_cpu_and_clears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let mut settings = Settings::default();
        settings.cpu_alert_cores = 1.0;
        settings.cpu_alert_duration_s = 5.0;
        settings.notifications = true;
        let start_ticks = (99.0 * clk_tck()) as u64;
        let delta = (2.0 * 6.0 * clk_tck()) as u64;

        tracker.update(
            snapshot(
                vec![chrome(600, 1, start_ticks, 0)],
                base_at,
                0.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        let session_id = tracker.session_views(&settings, base_wall)[0].id.clone();
        tracker.update(
            snapshot(
                vec![chrome(600, 1, start_ticks, delta)],
                base_at,
                6.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        let notices = tracker.update(
            snapshot(
                vec![chrome(600, 1, start_ticks, delta * 2)],
                base_at,
                12.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        assert!(notices
            .iter()
            .any(|notice| notice.title.contains("high CPU")));
        let high_alerts = tracker
            .events
            .iter()
            .filter(|event| event.kind == "high_cpu")
            .count();
        assert_eq!(high_alerts, 1);
        assert!(
            tracker.session_views(&settings, base_wall + 12.0)[0]
                .status
                .high_cpu
        );

        tracker.update(
            snapshot(
                vec![chrome(600, 1, start_ticks, delta * 3)],
                base_at,
                18.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        assert_eq!(
            tracker
                .events
                .iter()
                .filter(|event| event.kind == "high_cpu")
                .count(),
            1
        );

        tracker.update(
            snapshot(
                vec![chrome(600, 1, start_ticks, delta * 3)],
                base_at,
                24.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        let view = &tracker.session_views(&settings, base_wall + 24.0)[0];
        assert!(!view.status.high_cpu);
        assert!(tracker
            .events
            .iter()
            .any(|event| event.kind == "high_cpu_cleared"));
        assert!(tracker.sessions.contains_key(&session_id));
    }

    #[test]
    fn session_mute_suppresses_high_cpu_notification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let mut settings = Settings::default();
        settings.cpu_alert_duration_s = 5.0;
        settings.notifications = true;
        let start_ticks = (99.0 * clk_tck()) as u64;
        let delta = (2.0 * 6.0 * clk_tck()) as u64;
        tracker.update(
            snapshot(
                vec![chrome(800, 1, start_ticks, 0)],
                base_at,
                0.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        tracker.update(
            snapshot(
                vec![chrome(800, 1, start_ticks, delta)],
                base_at,
                6.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        let session_id = tracker.session_views(&settings, base_wall)[0].id.clone();
        assert!(tracker.mute_session(&session_id, 30));
        let notices = tracker.update(
            snapshot(
                vec![chrome(800, 1, start_ticks, delta * 2)],
                base_at,
                12.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        assert!(notices.is_empty());
        assert!(tracker.events.iter().any(|event| event.kind == "high_cpu"));
    }

    #[test]
    fn global_mute_keeps_alert_event_without_notification() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = Storage::open(dir.path()).expect("storage");
        let mut tracker = Tracker::new("boot".to_string(), 8);
        let base_at = Instant::now();
        let base_wall = tracker.started_wall + 1.0;
        let mut settings = Settings::default();
        settings.cpu_alert_duration_s = 5.0;
        settings.global_mute = true;
        settings.notifications = true;
        let start_ticks = (99.0 * clk_tck()) as u64;
        let delta = (2.0 * 6.0 * clk_tck()) as u64;
        tracker.update(
            snapshot(
                vec![chrome(700, 1, start_ticks, 0)],
                base_at,
                0.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        tracker.update(
            snapshot(
                vec![chrome(700, 1, start_ticks, delta)],
                base_at,
                6.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        let notices = tracker.update(
            snapshot(
                vec![chrome(700, 1, start_ticks, delta * 2)],
                base_at,
                12.0,
                base_wall,
            ),
            &settings,
            &mut store,
        );
        assert!(notices.is_empty());
        assert!(tracker.events.iter().any(|event| event.kind == "high_cpu"));
    }
}
