use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

use crate::model::{ProcSnapshot, ProcessInfo};

pub const BROWSER_EXECUTABLES: &[&str] = &[
    "chrome",
    "google-chrome",
    "google-chrome-stable",
    "google-chrome-beta",
    "google-chrome-unstable",
    "google-chrome-canary",
    "chromium",
    "chromium-browser",
    "chrome-headless-shell",
    "chromium-headless-shell",
    "headless_shell",
    "headless-shell",
    "microsoft-edge",
    "microsoft-edge-stable",
    "microsoft-edge-beta",
    "microsoft-edge-dev",
    "msedge",
    "msedge-stable",
    "edge",
];

static HEADLESS_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|\s)--headless(?:=(?:new|old|chrome|true))?(?:\s|$)")
        .expect("valid headless regex")
});
static TYPE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|\s)--type=([A-Za-z0-9._-]+)").expect("valid type regex"));

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserEvidence {
    pub is_browser: bool,
    pub is_root: bool,
    pub is_headless: bool,
    pub role: Option<String>,
    pub family: String,
    pub executable: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CdpEndpoint {
    pub mode: String,
    pub port: Option<u16>,
    pub reason: Option<String>,
}

pub fn is_browser_executable(proc: &ProcessInfo) -> bool {
    BROWSER_EXECUTABLES.contains(&proc.executable_basename().as_str())
}

pub fn process_role(proc: &ProcessInfo) -> Option<String> {
    for arg in &proc.argv {
        if let Some(value) = arg.strip_prefix("--type=") {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    TYPE_RE
        .captures(&proc.command())
        .and_then(|caps| caps.get(1).map(|value| value.as_str().to_string()))
}

pub fn has_headless_evidence(proc: &ProcessInfo) -> bool {
    let name = proc.executable_basename();
    if name.contains("headless") {
        return true;
    }
    proc.argv.iter().any(|arg| {
        arg == "--headless"
            || arg == "--headless=new"
            || arg == "--headless=old"
            || arg == "--headless=chrome"
            || arg == "--headless=true"
    }) || HEADLESS_RE.is_match(&proc.command())
}

pub fn browser_family(proc: &ProcessInfo) -> String {
    let name = proc.executable_basename();
    if name.contains("edge") || name.contains("msedge") {
        "Microsoft Edge".to_string()
    } else if name.contains("headless") && name.contains("shell") {
        if name.contains("chromium") {
            "Chromium Headless Shell".to_string()
        } else {
            "Chrome Headless Shell".to_string()
        }
    } else if name.contains("chromium") {
        "Chromium".to_string()
    } else {
        "Google Chrome".to_string()
    }
}

pub fn evidence_for(proc: &ProcessInfo) -> BrowserEvidence {
    if !is_browser_executable(proc) {
        return BrowserEvidence {
            is_browser: false,
            is_root: false,
            is_headless: false,
            role: None,
            family: "Unknown".to_string(),
            executable: proc.executable_basename(),
        };
    }
    let role = process_role(proc);
    let headless = has_headless_evidence(proc);
    BrowserEvidence {
        is_browser: true,
        is_root: role.is_none() && headless,
        is_headless: headless,
        role,
        family: browser_family(proc),
        executable: proc.executable_basename(),
    }
}

pub fn is_headless_root(proc: &ProcessInfo) -> bool {
    evidence_for(proc).is_root
}

pub fn browser_roots(snapshot: &ProcSnapshot) -> Vec<ProcessInfo> {
    let candidates = snapshot
        .processes
        .values()
        .filter(|proc| is_headless_root(proc))
        .cloned()
        .collect::<Vec<_>>();
    if candidates.len() <= 1 {
        return candidates;
    }
    let root_pids = candidates
        .iter()
        .map(|proc| proc.pid)
        .collect::<std::collections::HashSet<_>>();
    let mut roots = Vec::new();
    for proc in candidates {
        let mut current = proc.clone();
        let mut seen = std::collections::HashSet::new();
        let mut nested = false;
        while current.ppid > 1 && seen.insert(current.ppid) {
            if root_pids.contains(&current.ppid) {
                nested = true;
                break;
            }
            let Some(parent) = snapshot.processes.get(&current.ppid) else {
                break;
            };
            current = parent.clone();
        }
        if !nested {
            roots.push(proc);
        }
    }
    roots
}

fn switch_value(proc: &ProcessInfo, switch: &str) -> Option<String> {
    for (index, arg) in proc.argv.iter().enumerate() {
        if arg == switch {
            return proc.argv.get(index + 1).cloned();
        }
        if !arg.chars().any(char::is_whitespace) {
            if let Some(value) = arg.strip_prefix(&format!("{switch}=")) {
                return Some(value.to_string());
            }
        }
    }
    let pattern = format!(
        r#"(?:^|\s){}(?:=|\s+)(?:"([^"]*)"|'([^']*)'|(\S+))"#,
        regex::escape(switch)
    );
    let regex = Regex::new(&pattern).ok()?;
    let command = proc.command();
    let caps = regex.captures(&command)?;
    caps.iter()
        .skip(1)
        .flatten()
        .next()
        .map(|value| value.as_str().to_string())
}

fn switch_present(proc: &ProcessInfo, switch: &str) -> bool {
    proc.argv
        .iter()
        .any(|arg| arg == switch || arg.starts_with(&format!("{switch}=")))
        || Regex::new(&format!(r"(?:^|\s){}(?:=|\s|$)", regex::escape(switch)))
            .is_ok_and(|regex| regex.is_match(&proc.command()))
}

fn active_port_from_profile(user_data_dir: Option<String>) -> Option<u16> {
    let path = Path::new(&user_data_dir?).join("DevToolsActivePort");
    let text = fs::read_to_string(path).ok()?;
    text.lines()
        .next()?
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|value| *value > 0)
}

pub fn cdp_endpoint(proc: &ProcessInfo) -> CdpEndpoint {
    let pipe = switch_present(proc, "--remote-debugging-pipe");
    let raw_port = switch_value(proc, "--remote-debugging-port");
    let mut port = raw_port
        .as_deref()
        .and_then(|value| value.parse::<i64>().ok());
    if port == Some(0) {
        port = active_port_from_profile(switch_value(proc, "--user-data-dir")).map(i64::from);
    }
    if let Some(value) = port.filter(|value| (1..=65535).contains(value)) {
        return CdpEndpoint {
            mode: "tcp".to_string(),
            port: Some(value as u16),
            reason: None,
        };
    }
    if pipe {
        return CdpEndpoint {
            mode: "pipe".to_string(),
            port: None,
            reason: Some(
                "Uses --remote-debugging-pipe; there is no independent TCP endpoint to attach to."
                    .to_string(),
            ),
        };
    }
    if raw_port.as_deref() == Some("0") {
        return CdpEndpoint {
            mode: "none".to_string(),
            port: None,
            reason: Some(
                "CDP was requested on an ephemeral port, but DevToolsActivePort is not readable."
                    .to_string(),
            ),
        };
    }
    CdpEndpoint {
        mode: "none".to_string(),
        port: None,
        reason: Some("The browser was not started with a CDP debugging endpoint.".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};
    use std::time::Instant;

    fn proc(pid: u32, ppid: u32, exe: &str, argv: &[&str]) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            start_ticks: 1,
            cpu_ticks: 0,
            rss_bytes: 0,
            state: "S".to_string(),
            name: exe.rsplit('/').next().unwrap_or(exe).to_string(),
            exe: exe.to_string(),
            argv: argv.iter().map(|value| value.to_string()).collect(),
            uid: Some(1000),
            cgroup: None,
            boot_id: "boot".to_string(),
        }
    }

    fn snapshot(processes: Vec<ProcessInfo>) -> ProcSnapshot {
        let pids = processes.iter().map(|proc| proc.pid).collect();
        ProcSnapshot {
            boot_id: "boot".to_string(),
            uptime_s: 1.0,
            processes: processes
                .into_iter()
                .map(|proc| (proc.pid, proc))
                .collect::<HashMap<_, _>>(),
            captured_at: Instant::now(),
            captured_wall: 1.0,
            pids_seen: pids,
            unreadable: HashSet::new(),
            zombie_pids: HashSet::new(),
            scan_ok: true,
            error: None,
            online_cpus: 8,
        }
    }

    #[test]
    fn excludes_gui_and_children() {
        let gui = proc(
            10,
            1,
            "/opt/google/chrome/chrome",
            &["--user-data-dir=/tmp/profile"],
        );
        assert!(!is_headless_root(&gui));
        let child = proc(
            11,
            10,
            "/opt/google/chrome/chrome",
            &["--type=renderer", "--headless=new"],
        );
        assert!(!evidence_for(&child).is_root);
        assert_eq!(evidence_for(&child).role.as_deref(), Some("renderer"));
    }

    #[test]
    fn detects_headless_roots_and_nested_roots() {
        let root = proc(20, 1, "/opt/google/chrome/chrome", &["--headless=new"]);
        assert!(is_headless_root(&root));
        let nested = proc(21, 20, "/opt/google/chrome/chrome", &["--headless=new"]);
        let roots = browser_roots(&snapshot(vec![root, nested]));
        assert_eq!(
            roots.iter().map(|proc| proc.pid).collect::<Vec<_>>(),
            vec![20]
        );
    }

    #[test]
    fn parses_cdp_variants_and_monolithic_cmdline() {
        let pipe = proc(
            30,
            1,
            "/opt/google/chrome/chrome",
            &["--headless", "--remote-debugging-pipe"],
        );
        assert_eq!(cdp_endpoint(&pipe).mode, "pipe");
        let tcp = proc(
            31,
            1,
            "/opt/google/chrome/chrome",
            &["--remote-debugging-port=9222"],
        );
        assert_eq!(
            (cdp_endpoint(&tcp).mode.as_str(), cdp_endpoint(&tcp).port),
            ("tcp", Some(9222))
        );
        let dir = tempfile::tempdir().expect("tempdir");
        fs::write(dir.path().join("DevToolsActivePort"), "7890\n/browser\n").expect("write");
        let monolithic = proc(
            32,
            1,
            "/opt/google/chrome/chrome",
            &[&format!(
                "--remote-debugging-port=0 --user-data-dir={}",
                dir.path().display()
            )],
        );
        let endpoint = cdp_endpoint(&monolithic);
        assert_eq!((endpoint.mode.as_str(), endpoint.port), ("tcp", Some(7890)));
    }
}
