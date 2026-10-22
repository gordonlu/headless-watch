use std::collections::{HashMap, HashSet};
use std::time::Instant;

use serde::{Deserialize, Serialize};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessIdentity {
    pub boot_id: String,
    pub pid: u32,
    pub start_ticks: u64,
}

impl ProcessIdentity {
    pub fn key(&self) -> String {
        format!("{}:{}:{}", self.boot_id, self.pid, self.start_ticks)
    }
}

#[derive(Clone, Debug)]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub start_ticks: u64,
    pub cpu_ticks: u64,
    pub rss_bytes: u64,
    pub state: String,
    pub name: String,
    pub exe: String,
    pub argv: Vec<String>,
    pub uid: Option<u32>,
    pub cgroup: Option<String>,
    pub boot_id: String,
}

impl ProcessInfo {
    pub fn identity(&self) -> ProcessIdentity {
        ProcessIdentity {
            boot_id: self.boot_id.clone(),
            pid: self.pid,
            start_ticks: self.start_ticks,
        }
    }

    pub fn command(&self) -> String {
        if self.argv.is_empty() {
            if self.exe.is_empty() {
                self.name.clone()
            } else {
                self.exe.clone()
            }
        } else {
            self.argv.join(" ")
        }
    }

    pub fn executable_basename(&self) -> String {
        let exe = self.exe.strip_suffix(" (deleted)").unwrap_or(&self.exe);
        exe.rsplit(['/', '\\'])
            .next()
            .unwrap_or(exe)
            .to_ascii_lowercase()
    }
}

#[derive(Clone, Debug)]
pub struct ProcSnapshot {
    pub boot_id: String,
    pub uptime_s: f64,
    pub processes: HashMap<u32, ProcessInfo>,
    pub captured_at: Instant,
    pub captured_wall: f64,
    pub pids_seen: HashSet<u32>,
    pub unreadable: HashSet<u32>,
    pub zombie_pids: HashSet<u32>,
    pub scan_ok: bool,
    pub error: Option<String>,
    pub online_cpus: usize,
}

impl ProcSnapshot {
    pub fn empty(error: String, online_cpus: usize, boot_id: String) -> Self {
        Self {
            boot_id,
            uptime_s: 0.0,
            processes: HashMap::new(),
            captured_at: Instant::now(),
            captured_wall: crate::util::now_wall(),
            pids_seen: HashSet::new(),
            unreadable: HashSet::new(),
            zombie_pids: HashSet::new(),
            scan_ok: false,
            error: Some(error),
            online_cpus: online_cpus.max(1),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StatFields {
    pub comm: String,
    pub state: String,
    pub ppid: u32,
    pub utime: u64,
    pub stime: u64,
    pub starttime: u64,
    pub rss_pages: u64,
}

impl StatFields {
    pub fn cpu_ticks(&self) -> u64 {
        self.utime.saturating_add(self.stime)
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct CpuValue {
    pub cores: f64,
    pub single_core_pct: f64,
    pub machine_pct: f64,
}

impl CpuValue {
    pub fn from_delta(delta_ticks: i64, elapsed_s: f64, online_cpus: usize) -> Self {
        if elapsed_s <= 0.0 || online_cpus == 0 {
            return Self::default();
        }
        let cores = (delta_ticks.max(0) as f64 / crate::procfs::clk_tck() / elapsed_s).max(0.0);
        Self {
            cores,
            single_core_pct: cores * 100.0,
            machine_pct: (cores / online_cpus as f64) * 100.0,
        }
    }
}

pub fn machine_pct(cores: f64, online_cpus: usize) -> f64 {
    if online_cpus == 0 {
        0.0
    } else {
        cores.max(0.0) / online_cpus as f64 * 100.0
    }
}
