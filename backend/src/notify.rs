use std::process::{Command, Stdio};

use crate::util::truncate_chars;

pub fn available() -> bool {
    Command::new("notify-send")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn send(title: &str, body: &str) {
    let _ = Command::new("notify-send")
        .arg("--app-name=Headless Watch")
        .arg(truncate_chars(title, 120))
        .arg(truncate_chars(body, 300))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}
