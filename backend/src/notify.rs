use std::process::{Command, Stdio};

use crate::util::truncate_chars;

fn dashboard_url(session_id: Option<&str>) -> Option<String> {
    let base = std::env::var("HEADLESS_WATCH_DASHBOARD_URL").ok()?;
    let session = session_id?;
    Some(format!("{base}#live?session={session}"))
}

pub fn available() -> bool {
    Command::new("notify-send")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

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
                    let _ = Command::new("xdg-open").arg(url).spawn();
                }
            }
        } else {
            let _ = command.stdout(Stdio::null()).stderr(Stdio::null()).spawn();
        }
    });
}
