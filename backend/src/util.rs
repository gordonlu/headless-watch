use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, SecondsFormat, Utc};

pub fn now_wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn iso_timestamp(timestamp: Option<f64>) -> Option<String> {
    let timestamp = timestamp?;
    let seconds = timestamp.trunc() as i64;
    let nanos = ((timestamp.fract().abs()) * 1_000_000_000.0) as u32;
    DateTime::<Utc>::from_timestamp(seconds, nanos)
        .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
}

pub fn format_duration(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;
    if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {secs}s")
    } else {
        format!("{secs}s")
    }
}

pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    let mut result: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    result.push('…');
    result
}

pub fn home_dir() -> String {
    #[cfg(windows)]
    return std::env::var("USERPROFILE").unwrap_or_default();
    #[cfg(not(windows))]
    std::env::var("HOME").unwrap_or_else(|_| "/".to_string())
}

/// Open the local dashboard using the registered browser without a command shell.
pub fn open_browser(url: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        let wide = |value: &str| -> Vec<u16> {
            std::ffi::OsStr::new(value)
                .encode_wide()
                .chain(Some(0))
                .collect()
        };
        let operation = wide("open");
        let target = wide(url);
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                operation.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
            )
        };
        if result as isize <= 32 {
            return Err(std::io::Error::other(format!(
                "Windows browser launch failed ({})",
                result as isize
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let status = std::process::Command::new("xdg-open").arg(url).status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other("xdg-open failed"))
        }
    }
}
