//! Windows process collector. No shell, WMI, or subprocess is needed per sample.
//! `start_ticks` preserves exact 100 ns creation ticks relative to this boot.
//! CPU ticks are also 100 ns units. Relative values fit JavaScript safe integers.
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::Path;
use std::ptr::{null_mut, read_unaligned};
use std::sync::OnceLock;
use std::time::Instant;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_NO_MORE_FILES, FILETIME, HANDLE,
    INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::{
    GetLengthSid, GetTokenInformation, TokenUser, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::SystemInformation::GetTickCount64;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW,
    WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
};
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

use crate::model::{ProcSnapshot, ProcessInfo, StatFields};
use crate::util::now_wall;

pub const DEFAULT_PROC_ROOT: &str = "windows-native";
const FILETIME_UNIX_EPOCH: f64 = 11_644_473_600.0;
const MAX_COMMAND_BYTES: usize = 1_048_576;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *const u16,
}

#[repr(C)]
struct ProcessBasicInformation {
    exit_status: i32,
    peb: *mut c_void,
    affinity_mask: usize,
    priority: i32,
    pid: usize,
    parent_pid: usize,
}

#[repr(C)]
struct SystemTimeOfDayInformation {
    boot_time: u64,
    current_time: u64,
    time_zone_bias: i64,
    time_zone_id: u32,
    reserved: u32,
    boot_time_bias: u64,
    sleep_time_bias: u64,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationProcess(
        process: HANDLE,
        class: u32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
    fn NtQuerySystemInformation(
        class: u32,
        information: *mut c_void,
        length: u32,
        returned: *mut u32,
    ) -> i32;
}

pub(crate) struct OwnedHandle(pub HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl OwnedHandle {
    fn new(handle: HANDLE) -> Option<Self> {
        (!handle.is_null() && handle != INVALID_HANDLE_VALUE).then_some(Self(handle))
    }
}

pub(crate) fn open_process(pid: u32, access: u32) -> Result<OwnedHandle, std::io::Error> {
    OwnedHandle::new(unsafe { OpenProcess(access, 0, pid) })
        .ok_or_else(std::io::Error::last_os_error)
}

fn filetime(value: FILETIME) -> u64 {
    (u64::from(value.dwHighDateTime) << 32) | u64::from(value.dwLowDateTime)
}

fn process_times(handle: HANDLE) -> Option<(u64, u64, u64)> {
    let (mut created, mut exited, mut kernel, mut user) =
        unsafe { (zeroed(), zeroed(), zeroed(), zeroed()) };
    if unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } == 0 {
        return None;
    }
    Some((filetime(created), filetime(kernel), filetime(user)))
}

pub(crate) fn process_creation_time(handle: HANDLE) -> Option<u64> {
    process_times(handle).map(|times| creation_ticks(times.0))
}

pub(crate) fn process_running(handle: HANDLE) -> bool {
    unsafe { WaitForSingleObject(handle, 0) == WAIT_TIMEOUT }
}

/// Copy the complete token SID, rather than hashing it into a Unix UID.
pub(crate) fn process_sid(handle: HANDLE) -> Option<Vec<u8>> {
    let mut token = null_mut();
    if unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = OwnedHandle::new(token)?;
    let mut required = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut required);
    }
    if required < size_of::<TOKEN_USER>() as u32 || required > 65_536 {
        return None;
    }
    // TOKEN_USER contains pointers and requires pointer alignment.
    let mut storage = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            storage.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return None;
    }
    let user = unsafe { &*(storage.as_ptr().cast::<TOKEN_USER>()) };
    let len = unsafe { GetLengthSid(user.User.Sid) } as usize;
    if len == 0 || len > 1024 {
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(user.User.Sid.cast::<u8>(), len) }.to_vec())
}

pub(crate) fn current_sid() -> Option<Vec<u8>> {
    static SID: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    SID.get_or_init(|| process_sid(unsafe { GetCurrentProcess() }))
        .clone()
}

/// Compare the complete user SID on an already-open process handle.
/// The caller must keep the handle valid for the duration of this call.
pub fn process_is_current_user(handle: HANDLE) -> bool {
    process_sid(handle)
        .zip(current_sid())
        .is_some_and(|(owner, current)| owner == current)
}

pub fn clk_tck() -> f64 {
    10_000_000.0
}

pub fn online_cpu_count() -> usize {
    std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .max(1)
}

fn boot_identity() -> &'static (String, u64) {
    static BOOT: OnceLock<(String, u64)> = OnceLock::new();
    BOOT.get_or_init(|| {
        // SystemTimeOfDayInformation starts with a stable boot FILETIME.
        // This class requires its exact 48-byte structure size; an oversized
        // generic buffer produces STATUS_INFO_LENGTH_MISMATCH on Windows 11.
        let mut information: SystemTimeOfDayInformation = unsafe { zeroed() };
        let status = unsafe {
            NtQuerySystemInformation(
                3,
                (&mut information as *mut SystemTimeOfDayInformation).cast(),
                size_of::<SystemTimeOfDayInformation>() as u32,
                null_mut(),
            )
        };
        if status >= 0 && information.boot_time != 0 {
            (
                format!("windows-{:016x}", information.boot_time),
                information.boot_time,
            )
        } else {
            // A fixed 2020 epoch preserves all 100 ns bits while keeping today's
            // identity values below JavaScript's 2^53 limit.
            ("windows-native".to_string(), 132_223_104_000_000_000)
        }
    })
}

fn creation_ticks(created: u64) -> u64 {
    created.saturating_sub(boot_identity().1)
}

pub fn read_boot_id(_proc_root: &Path) -> String {
    boot_identity().0.clone()
}

pub fn read_uptime(_proc_root: &Path) -> f64 {
    unsafe { GetTickCount64() as f64 / 1000.0 }
}

fn wide_string(value: &[u16]) -> String {
    let end = value
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(value.len());
    String::from_utf16_lossy(&value[..end])
}

fn executable(handle: HANDLE) -> Option<String> {
    let mut buffer = vec![0u16; 32_768];
    let mut length = buffer.len() as u32;
    if unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length) } == 0 {
        return None;
    }
    Some(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn split_command_line(command: &str) -> Vec<String> {
    if command.is_empty() {
        return Vec::new();
    }
    let wide = command.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut count = 0;
    let argv = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
    if argv.is_null() {
        return Vec::new();
    }
    let args = if (1..=65_536).contains(&count) {
        unsafe { std::slice::from_raw_parts(argv, count as usize) }
            .iter()
            .map(|pointer| {
                let mut length = 0;
                unsafe {
                    while length < MAX_COMMAND_BYTES / 2 && *pointer.add(length) != 0 {
                        length += 1;
                    }
                }
                String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(*pointer, length) })
            })
            .collect()
    } else {
        Vec::new()
    };
    unsafe {
        LocalFree(argv.cast());
    }
    args
}

pub(crate) fn command_line(handle: HANDLE) -> Vec<String> {
    // ProcessCommandLineInformation (Windows 8.1+) returns the complete UTF-16
    // command line with its UNICODE_STRING relocated into the caller's buffer.
    // Failure is deliberately empty: executable names alone must not identify a GUI browser as headless.
    let mut required = 0;
    unsafe {
        NtQueryInformationProcess(handle, 60, null_mut(), 0, &mut required);
    }
    if required < size_of::<UnicodeString>() as u32 || required as usize > MAX_COMMAND_BYTES {
        return Vec::new();
    }
    let mut buffer = vec![0usize; (required as usize).div_ceil(size_of::<usize>())];
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            60,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    };
    if status < 0 {
        return Vec::new();
    }
    let value = unsafe { read_unaligned(buffer.as_ptr().cast::<UnicodeString>()) };
    let start = buffer.as_ptr() as usize;
    let text = value.buffer as usize;
    let bytes = value.length as usize;
    if bytes % 2 != 0
        || text % 2 != 0
        || text < start
        || text
            .checked_add(bytes)
            .is_none_or(|end| end > start + buffer.len() * size_of::<usize>())
    {
        return Vec::new();
    }
    split_command_line(&String::from_utf16_lossy(unsafe {
        std::slice::from_raw_parts(value.buffer, bytes / 2)
    }))
}

pub(crate) fn parent_pid(handle: HANDLE) -> Option<u32> {
    let mut basic: ProcessBasicInformation = unsafe { zeroed() };
    let status = unsafe {
        NtQueryInformationProcess(
            handle,
            0,
            (&mut basic as *mut ProcessBasicInformation).cast(),
            size_of::<ProcessBasicInformation>() as u32,
            null_mut(),
        )
    };
    (status >= 0).then_some(basic.parent_pid as u32)
}

fn process_from_handle(
    handle: HANDLE,
    pid: u32,
    ppid: u32,
    name: String,
    boot_id: &str,
    read_command: bool,
) -> Option<ProcessInfo> {
    let (created, kernel, user) = process_times(handle)?;
    if !process_running(handle) {
        return None;
    }
    let exe = executable(handle).unwrap_or_default();
    let name = exe
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(&name)
        .to_string();
    let browser = crate::browser::BROWSER_EXECUTABLES.contains(&name.to_ascii_lowercase().as_str());
    let mut memory: PROCESS_MEMORY_COUNTERS = unsafe { zeroed() };
    memory.cb = size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    let memory_ok = unsafe {
        K32GetProcessMemoryInfo(
            handle,
            &mut memory,
            size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    } != 0;
    let same_user = process_sid(handle)
        .zip(current_sid())
        .is_some_and(|(owner, current)| owner == current);
    Some(ProcessInfo {
        pid,
        ppid,
        start_ticks: creation_ticks(created),
        cpu_ticks: kernel.saturating_add(user),
        rss_bytes: if memory_ok {
            memory.WorkingSetSize as u64
        } else {
            0
        },
        state: "S".to_string(),
        name,
        exe,
        argv: if browser || read_command {
            command_line(handle)
        } else {
            Vec::new()
        },
        // Compatibility marker only, not a SID or a Unix UID. Termination always
        // rechecks the complete SID on the pinned process handle.
        uid: same_user.then_some(0),
        cgroup: None,
        boot_id: boot_id.to_string(),
    })
}

pub fn read_process(pid: u32, boot_id: &str, _proc_root: &Path) -> Option<ProcessInfo> {
    let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE).ok()?;
    process_from_handle(
        handle.0,
        pid,
        parent_pid(handle.0)?,
        String::new(),
        boot_id,
        true,
    )
}

pub fn read_uid(pid: u32, _proc_root: &Path) -> Option<u32> {
    let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION).ok()?;
    (process_sid(handle.0)? == current_sid()?).then_some(0)
}

pub fn read_cgroup(_pid: u32, _proc_root: &Path) -> Option<String> {
    None
}

pub fn scan(proc_root: &Path, boot_id: &str) -> ProcSnapshot {
    let captured_at = Instant::now();
    let captured_wall = now_wall();
    let Some(snapshot_handle) =
        OwnedHandle::new(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) })
    else {
        return ProcSnapshot::empty(
            format!(
                "Cannot enumerate Windows processes: {}",
                std::io::Error::last_os_error()
            ),
            online_cpu_count(),
            boot_id.to_string(),
        );
    };
    let mut entry: PROCESSENTRY32W = unsafe { zeroed() };
    entry.dwSize = size_of::<PROCESSENTRY32W>() as u32;
    if unsafe { Process32FirstW(snapshot_handle.0, &mut entry) } == 0 {
        return ProcSnapshot::empty(
            format!(
                "Cannot read Windows process snapshot: {}",
                std::io::Error::last_os_error()
            ),
            online_cpu_count(),
            boot_id.to_string(),
        );
    }
    let mut processes = HashMap::new();
    let mut pids_seen = HashSet::new();
    let mut unreadable = HashSet::new();
    let mut scan_ok = true;
    let mut scan_error = None;
    loop {
        let pid = entry.th32ProcessID;
        if pid != 0 {
            pids_seen.insert(pid);
            let proc = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE)
                .ok()
                .and_then(|handle| {
                    process_from_handle(
                        handle.0,
                        pid,
                        entry.th32ParentProcessID,
                        wide_string(&entry.szExeFile),
                        boot_id,
                        false,
                    )
                });
            if let Some(proc) = proc {
                // A browser whose command line cannot be read must not silently
                // disappear from a live snapshot; keep its session stale instead.
                if proc.argv.is_empty() && crate::browser::is_browser_executable(&proc) {
                    unreadable.insert(pid);
                }
                processes.insert(pid, proc);
            } else {
                unreadable.insert(pid);
            }
        }
        if unsafe { Process32NextW(snapshot_handle.0, &mut entry) } == 0 {
            let code = unsafe { GetLastError() };
            if code != ERROR_NO_MORE_FILES {
                scan_ok = false;
                scan_error = Some(format!(
                    "Windows process enumeration stopped early (error {code})."
                ));
            }
            break;
        }
    }
    // Enrich only browser ancestors with launcher command lines.
    let roots = processes
        .values()
        .filter(|proc| crate::browser::is_headless_root(proc))
        .map(|proc| proc.pid)
        .collect::<Vec<_>>();
    for root in roots {
        let mut pid = root;
        for _ in 0..16 {
            let Some(parent) = processes.get(&pid).map(|proc| proc.ppid) else {
                break;
            };
            if parent == 0 || parent == pid {
                break;
            }
            if let Some(proc) = processes.get_mut(&parent) {
                if proc.argv.is_empty() {
                    if let Ok(handle) = open_process(parent, PROCESS_QUERY_LIMITED_INFORMATION) {
                        if process_creation_time(handle.0) == Some(proc.start_ticks) {
                            proc.argv = command_line(handle.0);
                        }
                    }
                }
            }
            pid = parent;
        }
    }
    ProcSnapshot {
        boot_id: boot_id.to_string(),
        uptime_s: read_uptime(proc_root),
        processes,
        captured_at,
        captured_wall,
        pids_seen,
        unreadable,
        zombie_pids: HashSet::new(),
        scan_ok,
        error: scan_error,
        online_cpus: online_cpu_count(),
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
        if proc.pid == root_pid
            || snapshot
                .processes
                .get(&proc.ppid)
                .is_some_and(|parent| parent.start_ticks <= proc.start_ticks)
        {
            children.entry(proc.ppid).or_default().push(proc.pid);
        }
    }
    let mut queue = VecDeque::from([root_pid]);
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    while let Some(pid) = queue.pop_front() {
        if result.len() >= limit || !seen.insert(pid) {
            continue;
        }
        let Some(proc) = snapshot.processes.get(&pid) else {
            continue;
        };
        // Unknown ownership may still be observed, but can never be terminated.
        if uid.is_none() || proc.uid.is_none() || proc.uid == uid {
            result.push(proc.clone());
        }
        if let Some(kids) = children.get(&pid) {
            queue.extend(kids);
        }
    }
    result
}

pub fn identity_alive(
    snapshot: &ProcSnapshot,
    identity: &crate::model::ProcessIdentity,
) -> &'static str {
    match snapshot.processes.get(&identity.pid) {
        Some(proc)
            if proc.boot_id == identity.boot_id && proc.start_ticks == identity.start_ticks =>
        {
            "alive"
        }
        None if snapshot.unreadable.contains(&identity.pid) => "unreadable",
        _ => "exited",
    }
}

pub fn live_stat(pid: u32, start_ticks: u64, proc_root: &Path) -> Option<StatFields> {
    let proc = read_process(pid, "", proc_root)?;
    (proc.start_ticks == start_ticks).then_some(StatFields {
        comm: proc.name,
        state: proc.state,
        ppid: proc.ppid,
        utime: proc.cpu_ticks,
        stime: 0,
        starttime: proc.start_ticks,
        rss_pages: proc.rss_bytes / 4096,
    })
}

pub fn launched_at(snapshot: &ProcSnapshot, start_ticks: u64) -> f64 {
    if snapshot.boot_id.starts_with("windows-") {
        start_ticks.saturating_add(boot_identity().1) as f64 / clk_tck() - FILETIME_UNIX_EPOCH
    } else {
        // Keep fixture snapshots platform independent.
        snapshot.captured_wall - (snapshot.uptime_s - start_ticks as f64 / clk_tck()).max(0.0)
    }
}

pub fn read_pss(_pid: u32, _proc_root: &Path) -> Result<u64, String> {
    Err("PSS is not supported on Windows; working set is reported as RSS.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_windows_quoting_and_unicode() {
        let args = split_command_line(
            r#""C:\Program Files\Chrome\chrome.exe" --headless=new "--user-data-dir=C:\Users\测试\Chrome Profile""#,
        );
        assert_eq!(
            args,
            vec![
                r"C:\Program Files\Chrome\chrome.exe",
                "--headless=new",
                r"--user-data-dir=C:\Users\测试\Chrome Profile"
            ]
        );
    }

    #[test]
    fn samples_current_process_natively_with_stable_identity() {
        let root = Path::new(DEFAULT_PROC_ROOT);
        let boot = read_boot_id(root);
        let own = read_process(std::process::id(), &boot, root).expect("own process is accessible");
        assert_eq!(own.uid, Some(0));
        assert!(own.start_ticks > 0);
        assert!(
            own.start_ticks <= 9_007_199_254_740_991,
            "identity must round-trip through browser JSON numbers"
        );
        assert!(own.rss_bytes > 0);
        assert!(!own.argv.is_empty());
        assert_eq!(
            own.start_ticks,
            read_process(own.pid, &boot, root).unwrap().start_ticks
        );
        assert!(live_stat(own.pid, own.start_ticks + 1, root).is_none());
        assert!((now_wall() - launched_at(&scan(root, &boot), own.start_ticks)).abs() < 600.0);
        assert!(read_pss(own.pid, root)
            .unwrap_err()
            .contains("not supported"));
    }

    #[test]
    fn creation_identity_preserves_every_tick_and_reused_parent_is_excluded() {
        assert_eq!(size_of::<SystemTimeOfDayInformation>(), 48);
        let base = boot_identity().1;
        assert_eq!(creation_ticks(base + 1_000_000_001), 1_000_000_001);
        assert_eq!(creation_ticks(base + 1_000_000_002), 1_000_000_002);
        let path = Path::new(DEFAULT_PROC_ROOT);
        let boot = read_boot_id(path);
        let mut root = read_process(std::process::id(), &boot, path).unwrap();
        root.pid = 100;
        root.ppid = 1;
        root.start_ticks = 200;
        let mut child = root.clone();
        child.pid = 101;
        child.ppid = 100;
        child.start_ticks = 199;
        let mut snapshot = ProcSnapshot::empty(String::new(), 1, boot);
        snapshot.processes = HashMap::from([(root.pid, root), (child.pid, child)]);
        assert_eq!(
            descendants(&snapshot, 100, Some(0), 10)
                .iter()
                .map(|proc| proc.pid)
                .collect::<Vec<_>>(),
            vec![100]
        );
    }
}
