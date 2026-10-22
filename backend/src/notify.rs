#[cfg(not(windows))]
use std::process::{Command, Stdio};

#[cfg(not(windows))]
use crate::util::truncate_chars;

#[cfg(not(windows))]
fn dashboard_url(session_id: Option<&str>) -> Option<String> {
    let base = std::env::var("HEADLESS_WATCH_DASHBOARD_URL").ok()?;
    let session = session_id?;
    Some(format!("{base}#live?session={session}"))
}

#[cfg(not(windows))]
pub fn available() -> bool {
    Command::new("notify-send")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(not(windows))]
pub fn send(title: &str, body: &str, session_id: Option<&str>) {
    let url = dashboard_url(session_id);
    let mut command = Command::new("notify-send");
    command.arg("--app-name=Headless Watch");
    if url.is_some() {
        command.args(["--action=default=Open dashboard", "--wait"]);
    }
    command
        .arg(truncate_chars(title, 120))
        .arg(truncate_chars(body, 300));
    let url_for_thread = url;
    std::thread::spawn(move || {
        if let Some(url) = url_for_thread {
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            if let Ok(output) = command.output() {
                if String::from_utf8_lossy(&output.stdout).contains("default") {
                    let _ = crate::util::open_browser(&url);
                }
            }
        } else {
            let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        }
    });
}

// Windows alerts remain in the dashboard. Native toast registration is not
// implemented; do not spawn PowerShell or claim notifications are available.
#[cfg(windows)]
pub fn available() -> bool {
    false
}

#[cfg(windows)]
pub fn send(_title: &str, _body: &str, _session_id: Option<&str>) {}
