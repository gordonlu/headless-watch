use serde::{Deserialize, Serialize};

use crate::browser::process_role;
use crate::model::{ProcSnapshot, ProcessIdentity, ProcessInfo};
use crate::redaction::{redact_argv, redact_path};
use crate::util::now_wall;

const LAUNCHERS: &[(&str, &[&str])] = &[
    ("OpenCode", &["opencode"]),
    ("Codex", &["codex"]),
    ("Claude Code", &["claude"]),
];

const DRIVERS: &[(&str, &[&str])] = &[
    ("Playwright", &["playwright"]),
    ("Puppeteer", &["puppeteer"]),
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AncestorEvidence {
    pub identity: ProcessIdentity,
    pub pid: u32,
    pub name: String,
    pub ppid: u32,
    pub command: String,
    pub role: Option<String>,
    pub observed_at: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Attribution {
    pub launcher: String,
    pub launcher_confidence: String,
    pub launcher_identity: Option<ProcessIdentity>,
    pub launcher_name: Option<String>,
    pub launcher_pid: Option<u32>,
    pub driver: String,
    pub driver_confidence: String,
    pub driver_identity: Option<ProcessIdentity>,
    pub driver_name: Option<String>,
    pub driver_pid: Option<u32>,
    pub direct_parent: Option<ProcessIdentity>,
    pub direct_parent_name: Option<String>,
    pub direct_parent_pid: Option<u32>,
    pub parent_observable: bool,
    pub ancestors: Vec<AncestorEvidence>,
    pub cgroup_hint: Option<String>,
}

impl Attribution {
    pub fn source_summary(&self) -> String {
        if self.launcher == "Unknown" && self.driver == "Unknown" {
            return "Unknown origin".to_string();
        }
        match (self.launcher.as_str(), self.driver.as_str()) {
            ("Unknown", driver) => driver.to_string(),
            (launcher, "Unknown") => launcher.to_string(),
            (launcher, driver) => format!("{launcher} / {driver}"),
        }
    }

    pub fn origin_confidence(&self) -> &'static str {
        if self.launcher_confidence == "observed" {
            "observed"
        } else if self.launcher_confidence == "inferred" || self.driver_confidence == "inferred" {
            "inferred"
        } else {
            "unknown"
        }
    }

    pub fn watched_identity(&self) -> Option<(ProcessIdentity, String)> {
        if self.launcher_confidence == "observed" {
            if let Some(identity) = &self.launcher_identity {
                return Some((identity.clone(), self.launcher.clone()));
            }
        }
        if self.driver_confidence == "observed" {
            if let Some(identity) = &self.driver_identity {
                return Some((identity.clone(), self.driver.clone()));
            }
        }
        self.direct_parent
            .clone()
            .map(|identity| (identity, "Parent".to_string()))
    }
}

pub fn own_command(proc: &ProcessInfo) -> String {
    if proc.exe.is_empty() {
        proc.command()
    } else {
        format!("{} {}", proc.exe, proc.command())
            .trim()
            .to_string()
    }
}

fn label_tokens(text: &str, patterns: &[(&str, &[&str])]) -> Option<String> {
    let tokens = text
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
        .collect::<Vec<_>>();
    for (label, needles) in patterns {
        if needles
            .iter()
            .any(|needle| tokens.iter().any(|token| token == needle))
        {
            return Some((*label).to_string());
        }
    }
    None
}

fn match_label(
    ancestors: &[ProcessInfo],
    patterns: &[(&str, &[&str])],
) -> Option<(String, ProcessInfo)> {
    for ancestor in ancestors {
        let text = format!("{} {}", ancestor.name, own_command(ancestor));
        if let Some(label) = label_tokens(&text, patterns) {
            return Some((label, ancestor.clone()));
        }
    }
    None
}

fn cgroup_labels(cgroup: &str) -> (Option<String>, Option<String>) {
    (
        label_tokens(cgroup, LAUNCHERS),
        label_tokens(cgroup, DRIVERS),
    )
}

pub fn lineage(snapshot: &ProcSnapshot, pid: u32, limit: usize) -> Vec<ProcessInfo> {
    let mut result = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut current = pid;
    while current > 0 && result.len() < limit {
        if !seen.insert(current) {
            break;
        }
        let Some(proc) = snapshot.processes.get(&current) else {
            break;
        };
        result.push(proc.clone());
        if proc.ppid == 0 || proc.ppid == current {
            break;
        }
        current = proc.ppid;
    }
    result
}

pub fn build_attribution(
    snapshot: &ProcSnapshot,
    root: &ProcessInfo,
    cgroup_override: Option<&str>,
) -> Attribution {
    let chain = lineage(snapshot, root.pid, 16);
    let ancestors = chain.iter().skip(1).cloned().collect::<Vec<_>>();
    let observed_at = now_wall();
    let evidence = ancestors
        .iter()
        .map(|proc| AncestorEvidence {
            identity: proc.identity(),
            pid: proc.pid,
            name: proc.name.clone(),
            ppid: proc.ppid,
            command: redact_argv(&proc.argv, 220),
            role: process_role(proc),
            observed_at,
        })
        .collect::<Vec<_>>();
    let mut result = Attribution {
        launcher: "Unknown".to_string(),
        launcher_confidence: "unknown".to_string(),
        driver: "Unknown".to_string(),
        driver_confidence: "unknown".to_string(),
        ancestors: evidence,
        ..Attribution::default()
    };
    if let Some(direct) = ancestors.first() {
        result.direct_parent = Some(direct.identity());
        result.direct_parent_name = Some(direct.name.clone());
        result.direct_parent_pid = Some(direct.pid);
        result.parent_observable = true;
    }
    let cgroup = cgroup_override.or(root.cgroup.as_deref());
    result.cgroup_hint = cgroup.map(redact_path);

    if let Some((label, proc)) = match_label(&ancestors, LAUNCHERS) {
        result.launcher = label;
        result.launcher_confidence = "observed".to_string();
        result.launcher_identity = Some(proc.identity());
        result.launcher_name = Some(proc.name.clone());
        result.launcher_pid = Some(proc.pid);
    } else {
        let (label, _) = cgroup_labels(cgroup.unwrap_or_default());
        if let Some(label) = label {
            result.launcher = label;
            result.launcher_confidence = "inferred".to_string();
        }
    }

    if let Some((label, proc)) = match_label(&ancestors, DRIVERS) {
        result.driver = label;
        result.driver_confidence = "observed".to_string();
        result.driver_identity = Some(proc.identity());
        result.driver_name = Some(proc.name.clone());
        result.driver_pid = Some(proc.pid);
    } else {
        let (_, driver) = cgroup_labels(cgroup.unwrap_or_default());
        if let Some(driver) = driver {
            result.driver = driver;
            result.driver_confidence = "inferred".to_string();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;

    fn proc(pid: u32, ppid: u32, exe: &str, name: &str, argv: &[&str]) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            start_ticks: 10 + pid as u64,
            cpu_ticks: 0,
            rss_bytes: 0,
            state: "S".to_string(),
            name: name.to_string(),
            exe: exe.to_string(),
            argv: argv.iter().map(|value| value.to_string()).collect(),
            uid: Some(1000),
            cgroup: Some("/user.slice/app-opencode".to_string()),
            boot_id: "boot".to_string(),
        }
    }

    fn snapshot(processes: Vec<ProcessInfo>) -> ProcSnapshot {
        let pids = processes.iter().map(|proc| proc.pid).collect();
        ProcSnapshot {
            boot_id: "boot".to_string(),
            uptime_s: 10.0,
            processes: processes
                .into_iter()
                .map(|proc| (proc.pid, proc))
                .collect::<HashMap<_, _>>(),
            captured_at: Instant::now(),
            captured_wall: 1_000.0,
            pids_seen: pids,
            unreadable: std::collections::HashSet::new(),
            zombie_pids: std::collections::HashSet::new(),
            scan_ok: true,
            error: None,
            online_cpus: 8,
        }
    }

    #[test]
    fn separates_launcher_and_driver() {
        let root = proc(
            400,
            300,
            "/opt/google/chrome/chrome",
            "chrome",
            &["--headless=new"],
        );
        let driver = proc(
            300,
            200,
            "/usr/bin/node",
            "node",
            &["node", "/x/playwright-mcp/index.js"],
        );
        let launcher = proc(200, 1, "/usr/bin/opencode", "opencode", &["opencode"]);
        let attribution =
            build_attribution(&snapshot(vec![root.clone(), driver, launcher]), &root, None);
        assert_eq!(attribution.launcher, "OpenCode");
        assert_eq!(attribution.driver, "Playwright");
        assert_eq!(attribution.source_summary(), "OpenCode / Playwright");
        assert_eq!(attribution.direct_parent_pid, Some(300));
    }

    #[test]
    fn unknown_origin_is_not_invented() {
        let root = proc(
            500,
            499,
            "/opt/google/chrome/chrome",
            "chrome",
            &["--headless=new"],
        );
        let attribution = build_attribution(
            &snapshot(vec![root.clone()]),
            &root,
            Some("/user.slice/session-1.scope"),
        );
        assert_eq!(attribution.launcher, "Unknown");
        assert_eq!(attribution.driver, "Unknown");
        assert_eq!(attribution.origin_confidence(), "unknown");
    }
}
