use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use crate::model::{ProcSnapshot, ProcessInfo, StatFields};
use crate::util::now_wall;

pub const DEFAULT_PROC_ROOT: &str = "/proc";
const PAGE_SIZE: u64 = 4096;

pub fn clk_tck() -> f64 {
    static VALUE: OnceLock<f64> = OnceLock::new();
    *VALUE.get_or_init(|| {
        let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if ticks > 0 {
            ticks as f64
        } else {
            100.0
        }
    })
}

pub fn read_boot_id(proc_root: &Path) -> String {
    fs::read_to_string(proc_root.join("sys/kernel/random/boot_id"))
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|_| "unknown-boot".to_string())
}

pub fn read_uptime(proc_root: &Path) -> f64 {
    fs::read_to_string(proc_root.join("uptime"))
        .ok()
        .and_then(|value| value.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0)
}

pub fn parse_stat(raw: &str) -> Option<StatFields> {
    let open = raw.find('(')?;
    let close = raw.rfind(')')?;
    if close <= open {
        return None;
    }
    let comm = raw[open + 1..close].to_string();
    let mut fields = raw[close + 2..].split_whitespace();
    let state = fields.next()?.to_string();
    let ppid = fields.next()?.parse().ok()?;
    let mut utime = None;
    let mut stime = None;
    let mut starttime = None;
    let mut rss_pages = None;
    let mut index = 2usize;
    for field in fields {
        match index {
            11 => utime = field.parse().ok(),
            12 => stime = field.parse().ok(),
            19 => starttime = field.parse().ok(),
            21 => {
                rss_pages = field.parse().ok();
                break;
            }
            _ => {}
        }
        index += 1;
    }
    Some(StatFields {
        comm,
        state,
        ppid,
        utime: utime?,
        stime: stime?,
        starttime: starttime?,
        rss_pages: rss_pages?,
    })
}

pub fn read_stat(pid: u32, proc_root: &Path) -> Option<StatFields> {
    let raw = fs::read_to_string(proc_root.join(pid.to_string()).join("stat")).ok()?;
    parse_stat(&raw)
}

pub fn read_uid(pid: u32, proc_root: &Path) -> Option<u32> {
    fs::metadata(proc_root.join(pid.to_string()))
        .ok()
        .map(|metadata| metadata.uid())
}

pub fn read_cgroup(pid: u32, proc_root: &Path) -> Option<String> {
    let text = fs::read_to_string(proc_root.join(pid.to_string()).join("cgroup")).ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("0::") {
            let value = rest.trim();
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn read_argv(path: PathBuf) -> Vec<String> {
    let Ok(raw) = fs::read(path) else {
        return Vec::new();
    };
    raw.split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).to_string())
        .collect()
}

fn read_cmdline(pid: u32, proc_root: &Path) -> Vec<String> {
    read_argv(proc_root.join(pid.to_string()).join("cmdline"))
}

pub fn read_process(pid: u32, boot_id: &str, proc_root: &Path) -> Option<ProcessInfo> {
    let base = proc_root.join(pid.to_string());
    let raw = fs::read_to_string(base.join("stat")).ok()?;
    let stat = parse_stat(&raw)?;
    if stat.state == "Z" {
        return None;
    }
    let comm_lower = stat.comm.to_ascii_lowercase();
    let potential_browser = comm_lower.contains("chrome")
        || comm_lower.contains("chrom")
        || comm_lower.contains("edge")
        || comm_lower.contains("headless");
    let exe = if potential_browser {
        fs::read_link(base.join("exe"))
            .ok()
            .map(|value| value.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let exe = exe.strip_suffix(" (deleted)").unwrap_or(&exe).to_string();
    let name = if exe.is_empty() {
        stat.comm.clone()
    } else {
        exe.rsplit('/').next().unwrap_or(&exe).to_string()
    };
    // Only browser executables and browser ancestors need the full command line.
    // Reading it for every process is the dominant /proc cost on busy machines.
    let argv = if crate::browser::BROWSER_EXECUTABLES.contains(&name.to_ascii_lowercase().as_str())
    {
        read_cmdline(pid, proc_root)
    } else {
        Vec::new()
    };
    Some(ProcessInfo {
        pid,
        ppid: stat.ppid,
        start_ticks: stat.starttime,
        cpu_ticks: stat.cpu_ticks(),
        rss_bytes: stat.rss_pages.saturating_mul(PAGE_SIZE),
        state: stat.state,
        name,
        exe,
        argv,
        uid: None,
        cgroup: None,
        boot_id: boot_id.to_string(),
    })
}

pub fn parse_online_cpus(value: &str) -> Option<usize> {
    let mut total = 0usize;
    for part in value.trim().split(',') {
        if part.is_empty() {
            continue;
        }
        if let Some((start, end)) = part.split_once('-') {
            let start: usize = start.parse().ok()?;
            let end: usize = end.parse().ok()?;
            if end < start {
                return None;
            }
            total += end - start + 1;
        } else {
            part.parse::<usize>().ok()?;
            total += 1;
        }
    }
    (total > 0).then_some(total)
}

pub fn online_cpu_count() -> usize {
    static CACHE: OnceLock<Mutex<(std::time::Instant, usize)>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        Mutex::new((
            std::time::Instant::now() - std::time::Duration::from_secs(120),
            1,
        ))
    });
    let mut guard = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.0.elapsed() < std::time::Duration::from_secs(60) {
        return guard.1.max(1);
    }
    let parsed = fs::read_to_string("/sys/devices/system/cpu/online")
        .ok()
        .and_then(|value| parse_online_cpus(&value));
    let count = parsed
        .or_else(|| {
            std::thread::available_parallelism()
                .ok()
                .map(|value| value.get())
        })
        .unwrap_or(1)
        .max(1);
    guard.0 = std::time::Instant::now();
    guard.1 = count;
    count
}

pub fn scan(proc_root: &Path, boot_id: &str) -> ProcSnapshot {
    let captured_at = Instant::now();
    let captured_wall = now_wall();
    let uptime_s = read_uptime(proc_root);
    let entries = match fs::read_dir(proc_root) {
        Ok(value) => value,
        Err(err) => {
            return ProcSnapshot::empty(
                format!("Cannot read {}: {err}", proc_root.display()),
                online_cpu_count(),
                boot_id.to_string(),
            );
        }
    };
    let mut processes = HashMap::new();
    let mut pids_seen = HashSet::new();
    let mut unreadable = HashSet::new();
    let mut zombie_pids = HashSet::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|value| value.parse::<u32>().ok()) else {
            continue;
        };
        pids_seen.insert(pid);
        match read_process(pid, boot_id, proc_root) {
            Some(proc) => {
                processes.insert(pid, proc);
            }
            None => {
                let zombie = read_stat(pid, proc_root).is_some_and(|stat| stat.state == "Z");
                if zombie {
                    zombie_pids.insert(pid);
                } else {
                    unreadable.insert(pid);
                }
            }
        }
    }
    enrich_browser_metadata(&mut processes, proc_root);
    ProcSnapshot {
        boot_id: boot_id.to_string(),
        uptime_s,
        processes,
        captured_at,
        captured_wall,
        pids_seen,
        unreadable,
        zombie_pids,
        scan_ok: true,
        error: None,
        online_cpus: online_cpu_count(),
    }
}

fn enrich_browser_metadata(processes: &mut HashMap<u32, ProcessInfo>, proc_root: &Path) {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for proc in processes.values() {
        children.entry(proc.ppid).or_default().push(proc.pid);
    }
    let roots = processes
        .values()
        .filter(|proc| crate::browser::is_headless_root(proc))
        .map(|proc| proc.pid)
        .collect::<Vec<_>>();
    let mut subtree = HashSet::new();
    let mut queue = VecDeque::from(roots.clone());
    while let Some(pid) = queue.pop_front() {
        if !subtree.insert(pid) {
            continue;
        }
        if let Some(kids) = children.get(&pid) {
            queue.extend(kids.iter().copied());
        }
    }
    for pid in subtree {
        let uid = read_uid(pid, proc_root);
        let cgroup = read_cgroup(pid, proc_root);
        if let Some(proc) = processes.get_mut(&pid) {
            proc.uid = uid;
            proc.cgroup = cgroup;
        }
    }
    for root in roots {
        let mut current = root;
        let mut depth = 0;
        while depth < 16 {
            let Some(proc) = processes.get(&current) else {
                break;
            };
            let parent = proc.ppid;
            if parent == 0 || parent == current {
                break;
            }
            if let Some(parent_proc) = processes.get_mut(&parent) {
                if parent_proc.argv.is_empty() {
                    parent_proc.argv = read_cmdline(parent, proc_root);
                }
            }
            current = parent;
            depth += 1;
        }
    }
}

pub fn descendants(
    snapshot: &ProcSnapshot,
    root_pid: u32,
    uid: Option<u32>,
    limit: usize,
) -> Vec<ProcessInfo> {
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    for proc in snapshot.processes.values() {
        children.entry(proc.ppid).or_default().push(proc.pid);
    }
    let mut result = Vec::new();
    let mut queue = VecDeque::from([root_pid]);
    let mut seen = HashSet::new();
    while let Some(pid) = queue.pop_front() {
        if !seen.insert(pid) || result.len() >= limit {
            continue;
        }
        let Some(proc) = snapshot.processes.get(&pid) else {
            continue;
        };
        if uid.is_none() || proc.uid.is_none() || proc.uid == uid {
            result.push(proc.clone());
        }
        if let Some(items) = children.get(&pid) {
            queue.extend(items.iter().copied());
        }
    }
    result
}

pub fn identity_alive(
    snapshot: &ProcSnapshot,
    identity: &crate::model::ProcessIdentity,
) -> &'static str {
    let Some(proc) = snapshot.processes.get(&identity.pid) else {
        if snapshot.unreadable.contains(&identity.pid) {
            return "unreadable";
        }
        if snapshot.zombie_pids.contains(&identity.pid) {
            return "exited";
        }
        return "exited";
    };
    if proc.start_ticks == identity.start_ticks {
        "alive"
    } else {
        "exited"
    }
}

pub fn live_stat(pid: u32, start_ticks: u64, proc_root: &Path) -> Option<StatFields> {
    let stat = read_stat(pid, proc_root)?;
    if stat.starttime != start_ticks || matches!(stat.state.as_str(), "Z" | "X" | "x") {
        None
    } else {
        Some(stat)
    }
}

pub fn launched_at(snapshot: &ProcSnapshot, start_ticks: u64) -> f64 {
    let started = start_ticks as f64 / clk_tck();
    snapshot.captured_wall - (snapshot.uptime_s - started).max(0.0)
}

pub fn read_pss(pid: u32, proc_root: &Path) -> Result<u64, String> {
    let text = fs::read_to_string(proc_root.join(pid.to_string()).join("smaps_rollup"))
        .map_err(|err| err.to_string())?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Pss:") {
            let kb: u64 = rest
                .split_whitespace()
                .next()
                .ok_or_else(|| "missing PSS value".to_string())?
                .parse()
                .map_err(|_| "invalid PSS value".to_string())?;
            return Ok(kb.saturating_mul(1024));
        }
    }
    Err("PSS line not found".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stat_with_parentheses() {
        let raw = "4242 (chrome (x)) S 1 4242 4242 0 -1 4194304 0 0 0 0 100 50 0 0 20 0 1 0 900 12345 678 0 0";
        let stat = parse_stat(raw).expect("stat");
        assert_eq!(stat.comm, "chrome (x)");
        assert_eq!(stat.ppid, 1);
        assert_eq!(stat.cpu_ticks(), 150);
        assert_eq!(stat.starttime, 900);
        assert_eq!(stat.rss_pages, 678);
    }

    #[test]
    fn parses_online_cpu_ranges() {
        assert_eq!(parse_online_cpus("0-15"), Some(16));
        assert_eq!(parse_online_cpus("0-3,6,8-9\n"), Some(7));
        assert_eq!(parse_online_cpus("9-3"), None);
    }

    #[test]
    fn collapses_descendant_chain() {
        let procs = [100u32, 101, 102, 200]
            .into_iter()
            .map(|pid| {
                (
                    pid,
                    test_proc(
                        pid,
                        if pid == 100 {
                            1
                        } else if pid == 200 {
                            1
                        } else {
                            pid - 1
                        },
                    ),
                )
            })
            .collect();
        let snapshot = ProcSnapshot {
            boot_id: "boot".to_string(),
            uptime_s: 10.0,
            processes: procs,
            captured_at: Instant::now(),
            captured_wall: 1.0,
            pids_seen: HashSet::new(),
            unreadable: HashSet::new(),
            zombie_pids: HashSet::new(),
            scan_ok: true,
            error: None,
            online_cpus: 8,
        };
        let pids = descendants(&snapshot, 100, Some(1000), 100)
            .into_iter()
            .map(|proc| proc.pid)
            .collect::<Vec<_>>();
        assert_eq!(pids, vec![100, 101, 102]);
    }

    fn test_proc(pid: u32, ppid: u32) -> ProcessInfo {
        ProcessInfo {
            pid,
            ppid,
            start_ticks: 1,
            cpu_ticks: 0,
            rss_bytes: 0,
            state: "S".to_string(),
            name: "chrome".to_string(),
            exe: "/opt/google/chrome/chrome".to_string(),
            argv: Vec::new(),
            uid: Some(1000),
            cgroup: None,
            boot_id: "boot".to_string(),
        }
    }
}
