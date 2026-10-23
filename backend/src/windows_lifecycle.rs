//! Per-user Windows lifecycle. A random named event requests graceful shutdown;
//! no executable-name matching or process termination is used here.
use std::ffi::OsStr;
use std::fs::{self, File};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, FILETIME, HANDLE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, CreateMutexW, GetCurrentProcess, GetProcessTimes, OpenEventW, OpenProcess,
    QueryFullProcessImageNameW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, SYNCHRONIZATION_SYNCHRONIZE,
};

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

#[derive(Deserialize)]
struct InstallConfig {
    port: u16,
    data_dir: PathBuf,
    web_dir: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct Identity {
    pid: u32,
    created: u64,
    executable: PathBuf,
    event_name: String,
    port: u16,
}

fn installed() -> Result<(PathBuf, InstallConfig)> {
    let exe = std::env::current_exe()?;
    let root = exe
        .parent()
        .and_then(Path::parent)
        .context("cannot locate installation")?;
    let config = fs::read(root.join("install.json")).context("Windows installation not found; run install.ps1 first. For development, run without start/stop.")?;
    let config: InstallConfig = serde_json::from_slice(&config).context("invalid install.json")?;
    if config.port == 0 || !config.data_dir.is_absolute() || !config.web_dir.is_absolute() {
        bail!("invalid installation paths or port");
    }
    Ok((root.to_owned(), config))
}

fn created(handle: HANDLE) -> Result<u64> {
    let mut creation: FILETIME = unsafe { std::mem::zeroed() };
    let mut exit: FILETIME = unsafe { std::mem::zeroed() };
    let mut kernel: FILETIME = unsafe { std::mem::zeroed() };
    let mut user: FILETIME = unsafe { std::mem::zeroed() };
    if unsafe { GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot verify monitor creation time");
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

fn verified(identity: &Identity) -> Result<Handle> {
    let handle = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZATION_SYNCHRONIZE,
            0,
            identity.pid,
        )
    };
    if handle.is_null() {
        return Err(std::io::Error::last_os_error()).context("monitor is no longer running");
    }
    let handle = Handle(handle);
    if !headless_watch::procfs::process_is_current_user(handle.0) {
        bail!("refusing to signal a monitor owned by another user");
    }
    if created(handle.0)? != identity.created {
        bail!("stale monitor identity: PID has been reused");
    }
    let mut buffer = vec![0u16; 32768];
    let mut size = buffer.len() as u32;
    if unsafe {
        QueryFullProcessImageNameW(handle.0, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size)
    } == 0
    {
        return Err(std::io::Error::last_os_error()).context("cannot verify monitor executable");
    }
    let live_exe = PathBuf::from(String::from_utf16_lossy(&buffer[..size as usize]));
    // Compare canonical paths because current_exe may use the extended \\?\ form.
    if fs::canonicalize(live_exe)? != fs::canonicalize(&identity.executable)?
        || fs::canonicalize(&identity.executable)? != fs::canonicalize(std::env::current_exe()?)?
    {
        bail!("refusing to signal a different executable");
    }
    if unsafe { WaitForSingleObject(handle.0, 0) } == WAIT_OBJECT_0 {
        bail!("monitor has exited");
    }
    Ok(handle)
}

pub struct Instance {
    _mutex: Handle,
    event: Handle,
    path: PathBuf,
    event_name: String,
}

impl Instance {
    pub fn register(data_dir: &Path, port: u16) -> Result<Self> {
        fs::create_dir_all(data_dir)?;
        let canonical = fs::canonicalize(data_dir)?;
        let key = hex::encode(Sha256::digest(
            canonical.to_string_lossy().to_lowercase().as_bytes(),
        ));
        let mutex_name = wide(OsStr::new(&format!("Local\\HeadlessWatch-{key}")));
        let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
        let last_error = unsafe { GetLastError() };
        if mutex.is_null() {
            return Err(std::io::Error::last_os_error()).context("cannot create monitor lock");
        }
        let mutex = Handle(mutex);
        if last_error == ERROR_ALREADY_EXISTS {
            bail!(
                "another Headless Watch instance is already using {}",
                data_dir.display()
            );
        }
        let event_name = format!(
            "Local\\HeadlessWatch-Stop-{}",
            uuid::Uuid::new_v4().simple()
        );
        let name = wide(OsStr::new(&event_name));
        let event = unsafe { CreateEventW(std::ptr::null(), 1, 0, name.as_ptr()) };
        if event.is_null() {
            return Err(std::io::Error::last_os_error()).context("cannot create shutdown event");
        }
        let event = Handle(event);
        let identity = Identity {
            pid: std::process::id(),
            created: created(unsafe { GetCurrentProcess() })?,
            executable: std::env::current_exe()?,
            event_name: event_name.clone(),
            port,
        };
        let path = data_dir.join("monitor.json");
        fs::write(&path, serde_json::to_vec(&identity)?)?;
        Ok(Self {
            _mutex: mutex,
            event,
            path,
            event_name,
        })
    }

    pub fn event_handle(&self) -> usize {
        self.event.0 as usize
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // Do not remove metadata from a later run if external code changed it.
        if let Ok(data) = fs::read(&self.path) {
            if let Ok(identity) = serde_json::from_slice::<Identity>(&data) {
                if identity.event_name == self.event_name {
                    let _ = fs::remove_file(&self.path);
                }
            }
        }
    }
}

pub fn event_signaled(handle: usize) -> bool {
    unsafe { WaitForSingleObject(handle as HANDLE, 0) == WAIT_OBJECT_0 }
}

pub fn start() -> Result<()> {
    let (root, config) = installed()?;
    if let Ok(bytes) = fs::read(config.data_dir.join("monitor.json")) {
        if let Ok(identity) = serde_json::from_slice::<Identity>(&bytes) {
            if verified(&identity).is_ok() {
                println!(
                    "Headless Watch is already running at http://127.0.0.1:{}",
                    identity.port
                );
                return Ok(());
            }
        }
    }
    if super::local_health(config.port).is_some() {
        bail!(
            "port {} is occupied by another Headless Watch instance",
            config.port
        );
    }
    let logs = root.join("logs");
    fs::create_dir_all(&logs)?;
    let log = logs.join("headless-watch.log");
    if fs::metadata(&log).is_ok_and(|value| value.len() > 2 * 1024 * 1024) {
        let previous = logs.join("headless-watch.previous.log");
        let _ = fs::remove_file(&previous);
        fs::rename(&log, previous)?;
    }
    let output = File::options().append(true).create(true).open(&log)?;
    let mut child = Command::new(std::env::current_exe()?)
        .arg("--port")
        .arg(config.port.to_string())
        .arg("--data-dir")
        .arg(&config.data_dir)
        .arg("--web-dir")
        .arg(&config.web_dir)
        .current_dir(root)
        .creation_flags(0x08000000) // CREATE_NO_WINDOW
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait()? {
            bail!("monitor exited ({status}); check {}", log.display());
        }
        if super::local_health(config.port).is_some_and(|health| health.3 == child.id()) {
            println!("Headless Watch started at http://127.0.0.1:{}", config.port);
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
    bail!("monitor has not become ready; check {}", log.display())
}

pub fn stop() -> Result<()> {
    let (_, config) = installed()?;
    let path = config.data_dir.join("monitor.json");
    if !path.exists() {
        println!("Headless Watch is not running.");
        return Ok(());
    }
    let identity: Identity = serde_json::from_slice(&fs::read(&path)?)
        .context("invalid monitor identity; no process was signaled")?;
    let process = match verified(&identity) {
        Ok(process) => process,
        Err(err) => {
            println!("No verified monitor is running: {err}. No process was signaled.");
            return Ok(());
        }
    };
    let name = wide(OsStr::new(&identity.event_name));
    let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if event.is_null() {
        return Err(std::io::Error::last_os_error()).context("cannot open monitor shutdown event");
    }
    let event = Handle(event);
    if unsafe { SetEvent(event.0) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot request shutdown");
    }
    if unsafe { WaitForSingleObject(process.0, 10_000) } != WAIT_OBJECT_0 {
        bail!("graceful shutdown was requested but has not completed; no process was force-killed");
    }
    println!("Headless Watch stopped.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_lock_and_shutdown_event_are_scoped_to_data_directory() {
        let dir = tempfile::tempdir().unwrap();
        let instance = Instance::register(dir.path(), 8765).unwrap();
        assert!(Instance::register(dir.path(), 8765).is_err());
        assert!(!event_signaled(instance.event_handle()));
        let identity: Identity =
            serde_json::from_slice(&fs::read(dir.path().join("monitor.json")).unwrap()).unwrap();
        assert!(verified(&identity).is_ok());
        let name = wide(OsStr::new(&identity.event_name));
        let event = Handle(unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) });
        assert!(!event.0.is_null());
        assert_ne!(unsafe { SetEvent(event.0) }, 0);
        assert!(event_signaled(instance.event_handle()));
        drop(instance);
        assert!(!dir.path().join("monitor.json").exists());
    }

    #[test]
    fn reused_creation_time_never_matches() {
        let dir = tempfile::tempdir().unwrap();
        let _instance = Instance::register(dir.path(), 8765).unwrap();
        let mut identity: Identity =
            serde_json::from_slice(&fs::read(dir.path().join("monitor.json")).unwrap()).unwrap();
        identity.created += 1;
        assert!(verified(&identity).is_err());
    }
}
