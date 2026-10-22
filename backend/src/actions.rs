use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use tracing::debug;

use crate::browser::evidence_for;
use crate::model::ProcessInfo;
use crate::procfs::{descendants, live_stat, read_uid, scan, DEFAULT_PROC_ROOT};
use crate::util::truncate_chars;

const MAX_TARGETS: usize = 2000;

#[derive(Clone, Debug)]
pub struct TerminationRequest {
    pub session_id: String,
    pub boot_id: String,
    pub root_pid: u32,
    pub root_start_ticks: u64,
    pub session_uid: Option<u32>,
    pub mode: String,
    pub wait_s: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TerminationOutcome {
    pub ok: bool,
    pub result: String,
    pub message: String,
    pub mode: String,
    pub targets: usize,
    pub signaled: usize,
    pub survivors: usize,
    pub unauthorized: usize,
    pub skipped: usize,
    pub force_available: bool,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug)]
pub struct TerminationRejected(pub String);

impl std::fmt::Display for TerminationRejected {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TerminationRejected {}

fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn signal_process(pid: u32, signal: i32) -> Result<(), std::io::Error> {
    let result = unsafe { libc::kill(pid as i32, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn alive(pid: u32, start_ticks: u64, proc_root: &Path) -> bool {
    live_stat(pid, start_ticks, proc_root).is_some()
}

fn verify_root(
    proc_root: &Path,
    boot_id: &str,
    pid: u32,
    start_ticks: u64,
    session_uid: Option<u32>,
) -> Result<ProcessInfo, TerminationRejected> {
    let snapshot = scan(proc_root, boot_id);
    if !snapshot.scan_ok {
        return Err(TerminationRejected(
            "Unable to verify the process tree right now.".to_string(),
        ));
    }
    let Some(root) = snapshot.processes.get(&pid) else {
        return Err(TerminationRejected(
            "The browser root process no longer matches this session.".to_string(),
        ));
    };
    if root.start_ticks != start_ticks || root.boot_id != boot_id {
        return Err(TerminationRejected(
            "The browser root process no longer matches this session.".to_string(),
        ));
    }
    if root.uid != Some(current_uid())
        || session_uid
            .map(|uid| Some(uid) != root.uid)
            .unwrap_or(false)
    {
        return Err(TerminationRejected(
            "The browser process is not owned by this user.".to_string(),
        ));
    }
    if !evidence_for(root).is_root {
        return Err(TerminationRejected(
            "The process no longer matches headless browser evidence.".to_string(),
        ));
    }
    if root.pid == std::process::id() {
        return Err(TerminationRejected(
            "Refusing to signal the monitor itself.".to_string(),
        ));
    }
    Ok(root.clone())
}

pub fn terminate(
    request: &TerminationRequest,
    proc_root: &Path,
) -> Result<TerminationOutcome, TerminationRejected> {
    if request.mode != "term" && request.mode != "kill" {
        return Err(TerminationRejected(
            "Mode must be 'term' or 'kill'.".to_string(),
        ));
    }
    debug!(target: "headless_watch::actions", session = %request.session_id, "action: verifying root");
    let root = verify_root(
        proc_root,
        &request.boot_id,
        request.root_pid,
        request.root_start_ticks,
        request.session_uid,
    )?;
    debug!(target: "headless_watch::actions", session = %request.session_id, "action: root verified; scanning descendants");
    let snapshot = scan(proc_root, &request.boot_id);
    let uid = root.uid;
    let mut targets = descendants(&snapshot, root.pid, uid, MAX_TARGETS);
    targets.retain(|proc| proc.pid != std::process::id());
    targets.sort_by_key(|proc| proc.pid == root.pid);
    if targets.is_empty() {
        return Err(TerminationRejected(
            "No verified browser processes were found.".to_string(),
        ));
    }
    if targets.len() > MAX_TARGETS {
        return Err(TerminationRejected(
            "Browser process tree is unexpectedly large.".to_string(),
        ));
    }
    debug!(target: "headless_watch::actions", session = %request.session_id, targets = targets.len(), "action: signaling targets");
    let signal = if request.mode == "kill" {
        libc::SIGKILL
    } else {
        libc::SIGTERM
    };
    let mut signaled = 0usize;
    let mut unauthorized = 0usize;
    let mut skipped = 0usize;
    let mut errors = Vec::new();
    for target in &targets {
        let Some(current) = live_stat(target.pid, target.start_ticks, proc_root) else {
            skipped += 1;
            continue;
        };
        if current.state == "Z" {
            skipped += 1;
            continue;
        }
        if read_uid(target.pid, proc_root) != Some(current_uid()) {
            unauthorized += 1;
            continue;
        }
        match signal_process(target.pid, signal) {
            Ok(()) => signaled += 1,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => skipped += 1,
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => unauthorized += 1,
            Err(err) => {
                errors.push(err.kind().to_string());
                skipped += 1;
            }
        }
    }

    debug!(target: "headless_watch::actions", session = %request.session_id, signaled, "action: signals sent; waiting for root");
    let wait_s = request
        .wait_s
        .unwrap_or(if request.mode == "term" { 2.0 } else { 1.0 });
    let deadline = Instant::now() + Duration::from_secs_f64(wait_s.max(0.0));
    let mut root_alive = true;
    while Instant::now() < deadline {
        if !alive(root.pid, root.start_ticks, proc_root) {
            root_alive = false;
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    if root_alive {
        root_alive = alive(root.pid, root.start_ticks, proc_root);
    }
    let survivors = targets
        .iter()
        .filter(|target| alive(target.pid, target.start_ticks, proc_root))
        .count();
    debug!(target: "headless_watch::actions", session = %request.session_id, root_alive, survivors, "action: wait complete");
    let (result, message) = if signaled == 0 && unauthorized > 0 {
        (
            "unauthorized",
            "No processes were signaled: permission was denied.".to_string(),
        )
    } else if root_alive {
        (
            "still_running",
            format!(
                "{} was sent to {} verified process(es); the browser is still running ({} survivor(s)).",
                if request.mode == "kill" { "SIGKILL" } else { "SIGTERM" },
                signaled,
                survivors
            ),
        )
    } else if survivors > 0 {
        (
            "stopped_root",
            format!(
                "Browser root exited after {}; {} verified child process(es) remain.",
                if request.mode == "kill" {
                    "SIGKILL"
                } else {
                    "SIGTERM"
                },
                survivors
            ),
        )
    } else {
        (
            if request.mode == "kill" {
                "killed"
            } else {
                "stopped"
            },
            format!(
                "Browser stopped after {} ({} process(es) signaled).",
                if request.mode == "kill" {
                    "SIGKILL"
                } else {
                    "SIGTERM"
                },
                signaled
            ),
        )
    };
    let ok = matches!(result, "stopped" | "killed" | "stopped_root");
    Ok(TerminationOutcome {
        ok,
        result: result.to_string(),
        message: truncate_chars(&message, 240),
        mode: request.mode.clone(),
        targets: targets.len(),
        signaled,
        survivors,
        unauthorized,
        skipped,
        force_available: root_alive,
        details: if errors.is_empty() {
            serde_json::json!({})
        } else {
            serde_json::json!({"errors": errors})
        },
    })
}

pub fn default_proc_root() -> &'static Path {
    Path::new(DEFAULT_PROC_ROOT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(mode: &str, pid: u32, start_ticks: u64) -> TerminationRequest {
        TerminationRequest {
            session_id: "s-test".to_string(),
            boot_id: "boot".to_string(),
            root_pid: pid,
            root_start_ticks: start_ticks,
            session_uid: None,
            mode: mode.to_string(),
            wait_s: Some(0.0),
        }
    }

    #[test]
    fn rejects_invalid_mode_and_unverified_roots() {
        assert!(terminate(&request("pause", 1, 1), default_proc_root()).is_err());
        assert!(terminate(&request("term", 1, 1), default_proc_root()).is_err());
        assert!(terminate(&request("kill", std::process::id(), 1), default_proc_root()).is_err());
    }
}
