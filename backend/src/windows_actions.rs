//! Force termination is intentionally separate from Unix signal semantics.
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::System::Threading::{
    TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};

use crate::actions::{TerminationOutcome, TerminationRejected, TerminationRequest};
use crate::browser::evidence_for;
use crate::model::ProcessInfo;
use crate::procfs::{self, OwnedHandle};

const MAX_TARGETS: usize = 2000;

#[derive(Debug)]
enum VerifyFailure {
    Open(std::io::Error),
    Gone,
    Rejected,
    Owner,
}

fn verified_handle(target: &ProcessInfo, owner: &[u8]) -> Result<OwnedHandle, VerifyFailure> {
    let handle = procfs::open_process(
        target.pid,
        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE,
    )
    .map_err(VerifyFailure::Open)?;
    if target.pid == std::process::id() {
        return Err(VerifyFailure::Rejected);
    }
    if procfs::process_creation_time(handle.0) != Some(target.start_ticks)
        || !procfs::process_running(handle.0)
    {
        return Err(VerifyFailure::Gone);
    }
    if procfs::process_sid(handle.0).as_deref() != Some(owner) {
        return Err(VerifyFailure::Owner);
    }
    Ok(handle)
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
    let owner = procfs::current_sid().ok_or_else(|| {
        TerminationRejected("Unable to verify the current Windows user SID.".to_string())
    })?;
    if request.boot_id != procfs::read_boot_id(proc_root) {
        return Err(TerminationRejected(
            "The browser session belongs to a different Windows boot.".to_string(),
        ));
    }
    let snapshot = procfs::scan(proc_root, &request.boot_id);
    if !snapshot.scan_ok {
        return Err(TerminationRejected(
            "Unable to verify the process tree right now.".to_string(),
        ));
    }
    let root = snapshot.processes.get(&request.root_pid).filter(|root| {
        root.start_ticks == request.root_start_ticks
            && root.boot_id == request.boot_id
            && root.uid == Some(0)
            && request.session_uid.is_none_or(|uid| uid == 0)
            && root.pid != std::process::id()
            && evidence_for(root).is_root
    }).ok_or_else(|| TerminationRejected("The browser root identity, owner, or headless evidence no longer matches this session.".to_string()))?;

    // Opening once pins the kernel process object, preventing PID reuse between
    // the final checks and TerminateProcess. Root command line is re-read on it.
    let root_handle = verified_handle(root, &owner).map_err(|_| {
        TerminationRejected(
            "Unable to open the verified browser process for termination.".to_string(),
        )
    })?;
    if !evidence_for(&ProcessInfo {
        argv: procfs::command_line(root_handle.0),
        ..root.clone()
    })
    .is_root
    {
        return Err(TerminationRejected(
            "The browser no longer has verifiable headless evidence.".to_string(),
        ));
    }
    if request.mode == "term" {
        return Ok(TerminationOutcome {
            ok: false, result: "graceful_unavailable".to_string(),
            message: "Windows has no SIGTERM equivalent for headless Chromium. Use Force stop to terminate the verified browser tree; unsaved browser work will be lost.".to_string(),
            mode: request.mode.clone(), targets: 0, signaled: 0, survivors: 1,
            unauthorized: 0, skipped: 0, force_available: true,
            details: serde_json::json!({"platform": "windows", "graceful_supported": false}),
        });
    }
    let candidates = procfs::descendants(&snapshot, root.pid, root.uid, MAX_TARGETS + 1);
    if candidates.len() > MAX_TARGETS {
        return Err(TerminationRejected(
            "Browser process tree is unexpectedly large.".to_string(),
        ));
    }
    let mut verified: HashMap<u32, (ProcessInfo, OwnedHandle)> = HashMap::new();
    verified.insert(root.pid, (root.clone(), root_handle));
    let mut unauthorized = HashSet::new();
    let mut skipped = HashSet::new();
    // descendants is breadth first: a candidate's complete ancestry must already
    // be pinned and verified before admitting that child to the target list.
    for target in candidates.iter().filter(|target| target.pid != root.pid) {
        let Some((parent, _)) = verified.get(&target.ppid) else {
            skipped.insert(target.pid);
            continue;
        };
        if target.start_ticks < parent.start_ticks {
            skipped.insert(target.pid);
            continue;
        }
        match verified_handle(target, &owner) {
            Ok(handle) if procfs::parent_pid(handle.0) == Some(target.ppid) => {
                verified.insert(target.pid, (target.clone(), handle));
            }
            Ok(_) => {
                skipped.insert(target.pid);
            }
            Err(VerifyFailure::Open(err)) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                unauthorized.insert(target.pid);
            }
            Err(VerifyFailure::Owner) => {
                unauthorized.insert(target.pid);
            }
            Err(_) => {
                skipped.insert(target.pid);
            }
        }
    }
    let wait_s = request
        .wait_s
        .filter(|value| value.is_finite())
        .unwrap_or(1.0)
        .clamp(0.0, 30.0);
    let deadline = Instant::now() + Duration::from_secs_f64(wait_s);
    let mut signaled = 0;
    let mut errors = Vec::new();
    let mut sent = HashSet::new();
    let mut observed = candidates
        .iter()
        .map(|proc| proc.pid)
        .collect::<HashSet<_>>();
    let mut remaining = HashSet::new();
    let mut scan_confirmed = false;
    let mut tree_truncated = false;
    // Terminate the root first, preventing it from replacing children during
    // cleanup. Re-scan after each pass to catch children created while the
    // initial tree was being pinned. Keep even exited ancestor handles open:
    // their PIDs cannot be reused while validating a newly discovered child.
    for pass in 0..3 {
        let mut targets = verified.values().collect::<Vec<_>>();
        targets.sort_by_key(|(proc, _)| proc.pid != root.pid);
        for (target, handle) in targets {
            if !sent.insert(target.pid) || !procfs::process_running(handle.0) {
                continue;
            }
            if procfs::process_creation_time(handle.0) != Some(target.start_ticks)
                || !procfs::process_running(handle.0)
            {
                skipped.insert(target.pid);
                continue;
            }
            if procfs::process_sid(handle.0).as_deref() != Some(owner.as_slice()) {
                unauthorized.insert(target.pid);
                continue;
            }
            if unsafe { TerminateProcess(handle.0, 1) } != 0 {
                signaled += 1;
            } else {
                let error = std::io::Error::last_os_error();
                if !procfs::process_running(handle.0) {
                    skipped.insert(target.pid);
                } else if error.kind() == std::io::ErrorKind::PermissionDenied {
                    unauthorized.insert(target.pid);
                } else {
                    skipped.insert(target.pid);
                    errors.push(error.to_string());
                }
            }
        }
        while Instant::now() < deadline
            && verified
                .values()
                .any(|(_, handle)| procfs::process_running(handle.0))
        {
            thread::sleep(Duration::from_millis(10));
        }
        remaining = verified
            .values()
            .filter(|(_, handle)| procfs::process_running(handle.0))
            .map(|(proc, _)| proc.pid)
            .collect();
        let mut fresh = procfs::scan(proc_root, &request.boot_id);
        scan_confirmed = fresh.scan_ok;
        if !scan_confirmed {
            break;
        }
        let live_pids = fresh.processes.keys().copied().collect::<HashSet<_>>();
        for (proc, _) in verified.values() {
            fresh
                .processes
                .entry(proc.pid)
                .or_insert_with(|| proc.clone());
        }
        let current_tree = procfs::descendants(&fresh, root.pid, root.uid, MAX_TARGETS + 1);
        tree_truncated = current_tree.len() > MAX_TARGETS;
        if tree_truncated {
            break;
        }
        for proc in current_tree
            .iter()
            .filter(|proc| live_pids.contains(&proc.pid))
        {
            remaining.insert(proc.pid);
            observed.insert(proc.pid);
        }
        let before = verified.len();
        if pass < 2 {
            for target in current_tree
                .iter()
                .filter(|proc| live_pids.contains(&proc.pid))
            {
                if verified.contains_key(&target.pid) {
                    continue;
                }
                let Some((parent, _)) = verified.get(&target.ppid) else {
                    skipped.insert(target.pid);
                    continue;
                };
                if target.start_ticks < parent.start_ticks {
                    skipped.insert(target.pid);
                    continue;
                }
                match verified_handle(target, &owner) {
                    Ok(handle) if procfs::parent_pid(handle.0) == Some(target.ppid) => {
                        verified.insert(target.pid, (target.clone(), handle));
                    }
                    Ok(_) => {
                        skipped.insert(target.pid);
                    }
                    Err(VerifyFailure::Open(err))
                        if err.kind() == std::io::ErrorKind::PermissionDenied =>
                    {
                        unauthorized.insert(target.pid);
                    }
                    Err(VerifyFailure::Owner) => {
                        unauthorized.insert(target.pid);
                    }
                    Err(_) => {
                        skipped.insert(target.pid);
                    }
                }
            }
        }
        if verified.len() == before {
            break;
        }
    }
    let survivors = remaining.len();
    let root_alive = verified
        .get(&root.pid)
        .is_some_and(|(_, handle)| procfs::process_running(handle.0));
    let (result, message) = if root_alive {
        ("still_running", format!("TerminateProcess was applied to {signaled} verified process(es); the browser is still running ({survivors} survivor(s))."))
    } else if survivors > 0
        || !unauthorized.is_empty()
        || !skipped.is_empty()
        || !scan_confirmed
        || tree_truncated
    {
        ("stopped_root", format!("Browser root was force stopped. {survivors} observed survivor(s), {} denied and {} skipped process(es); complete tree cleanup is not confirmed.", unauthorized.len(), skipped.len()))
    } else {
        (
            "killed",
            format!(
                "Browser force stopped with TerminateProcess ({signaled} verified process(es)); the follow-up scan found no observed descendants."
            ),
        )
    };
    Ok(TerminationOutcome {
        ok: !root_alive,
        result: result.to_string(),
        message,
        mode: request.mode.clone(),
        targets: observed.len(),
        signaled,
        survivors,
        unauthorized: unauthorized.len(),
        skipped: skipped.len(),
        force_available: root_alive,
        details: serde_json::json!({"platform": "windows", "termination": "TerminateProcess", "graceful": false, "verified_targets": verified.len(), "verification_scope": "observed_descendants", "follow_up_scan": scan_confirmed, "tree_truncated": tree_truncated, "errors": errors}),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "helper process for the pinned-handle safety test"]
    fn child_wait() {
        thread::sleep(Duration::from_secs(30));
    }

    #[test]
    fn never_accepts_the_monitor_or_a_reused_pid() {
        let root = Path::new(procfs::DEFAULT_PROC_ROOT);
        let boot = procfs::read_boot_id(root);
        let mut own = procfs::read_process(std::process::id(), &boot, root).unwrap();
        assert!(verified_handle(&own, &procfs::current_sid().unwrap()).is_err());
        own.start_ticks += 1;
        assert!(verified_handle(&own, &procfs::current_sid().unwrap()).is_err());
    }

    #[test]
    fn verifies_a_pinned_child_and_rejects_changed_identity_and_owner() {
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                self.0.kill().ok();
                self.0.wait().ok();
            }
        }
        let child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "windows_actions::tests::child_wait", "--ignored"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("start test helper");
        let mut child = ChildGuard(child);
        let path = Path::new(procfs::DEFAULT_PROC_ROOT);
        let boot = procfs::read_boot_id(path);
        let target = procfs::read_process(child.0.id(), &boot, path).expect("child is readable");
        let owner = procfs::current_sid().unwrap();
        let pinned = verified_handle(&target, &owner).expect("valid child identity");
        assert_eq!(procfs::parent_pid(pinned.0), Some(std::process::id()));
        assert!(verified_handle(
            &ProcessInfo {
                start_ticks: target.start_ticks + 1,
                ..target.clone()
            },
            &owner
        )
        .is_err());
        assert!(verified_handle(&target, b"different-sid").is_err());
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "failed checks must leave the process running"
        );
    }
}
