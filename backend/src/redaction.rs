use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};
use url::Url;

use crate::util::{home_dir, truncate_chars};

const SECRET_SWITCHES: &[&str] = &[
    "--authorization",
    "--auth-token",
    "--cookie",
    "--cookies",
    "--header",
    "--headers",
    "--login-token",
    "--password",
    "--proxy-auth",
    "--proxy-server",
    "--secret",
    "--token",
    "--user-data-dir",
];

static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"https?://[^\s'"<>]+"#).expect("valid URL regex"));
fn is_secret_switch(value: &str) -> bool {
    SECRET_SWITCHES
        .iter()
        .any(|item| item.eq_ignore_ascii_case(value))
}

pub fn redact_url(value: &str) -> String {
    let Ok(parsed) = Url::parse(value) else {
        return value.to_string();
    };
    if !matches!(parsed.scheme(), "http" | "https" | "ws" | "wss") {
        return value.to_string();
    }
    let Some(host) = parsed.host_str() else {
        return "<redacted-url>".to_string();
    };
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let port = parsed
        .port()
        .map(|value| format!(":{value}"))
        .unwrap_or_default();
    let query = if parsed.query().is_some() {
        "?<redacted>"
    } else {
        ""
    };
    format!(
        "{}://{}{}{}{}",
        parsed.scheme(),
        host,
        port,
        parsed.path(),
        query
    )
}

fn redact_home(text: &str) -> String {
    let home = home_dir();
    if home == "/" || home.is_empty() {
        return text.to_string();
    }
    if cfg!(windows) {
        if let Ok(pattern) = RegexBuilder::new(&regex::escape(&home))
            .case_insensitive(true)
            .build()
        {
            return pattern.replace_all(text, "~").into_owned();
        }
    }
    text.replace(&home, "~")
}

fn clean_text(text: &str, max_length: usize) -> String {
    let cleaned: String = text
        .chars()
        .filter(|ch| !ch.is_control() || *ch == '\t')
        .collect();
    let cleaned = cleaned.replace(['\n', '\r'], " ");
    let cleaned = URL_RE.replace_all(&cleaned, |caps: &regex::Captures<'_>| redact_url(&caps[0]));
    let cleaned = redact_home(&cleaned);
    let collapsed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, max_length)
}

fn display_tokens(argv: &[String]) -> Vec<String> {
    if argv.len() == 1 && argv[0].contains(char::is_whitespace) {
        argv[0].split_whitespace().map(str::to_string).collect()
    } else {
        argv.to_vec()
    }
}

pub fn redact_argv(argv: &[String], max_length: usize) -> String {
    let mut tokens = Vec::new();
    let mut redact_next = false;
    for arg in display_tokens(argv) {
        if redact_next {
            tokens.push("<redacted>".to_string());
            redact_next = false;
            continue;
        }
        if let Some((switch, _)) = arg.split_once('=') {
            if switch.starts_with("--") && is_secret_switch(switch) {
                tokens.push(format!("{switch}=<redacted>"));
                continue;
            }
        }
        if arg.starts_with("--") && is_secret_switch(&arg) {
            tokens.push(arg);
            redact_next = true;
            continue;
        }
        tokens.push(arg);
    }
    clean_text(&tokens.join(" "), max_length)
}

pub fn redact_text(value: &str, max_length: usize) -> String {
    let tokens = value
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let joined = redact_argv(&tokens, max_length.saturating_add(64));
    if joined.chars().count() <= max_length {
        return joined;
    }
    truncate_chars(&joined, max_length)
}

pub fn redact_path(value: &str) -> String {
    let text: String = value.chars().filter(|ch| !ch.is_control()).collect();
    truncate_chars(&redact_home(&text), 240)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_url_query_and_userinfo() {
        assert_eq!(
            redact_url("https://user:pass@example.com/a/b?token=secret#frag"),
            "https://example.com/a/b?<redacted>"
        );
    }

    #[test]
    fn redacts_secret_switches_and_home() {
        let home = home_dir();
        let argv = vec![
            "chrome".to_string(),
            format!("--user-data-dir={home}/private profile"),
            "--remote-debugging-port=0".to_string(),
        ];
        let value = redact_argv(&argv, 300);
        assert!(value.contains("--user-data-dir=<redacted>"));
        assert!(value.contains("--remote-debugging-port=0"));
        assert!(!value.contains(&home));
    }

    #[test]
    fn preserves_argument_boundaries_for_spaced_windows_values() {
        let argv = vec![
            r"C:\Program Files\Chrome\chrome.exe".to_string(),
            "--headless=new".to_string(),
            r"--user-data-dir=C:\Users\John Doe\AppData\Local\Temp\profile".to_string(),
            "--header=Authorization: Bearer secret".to_string(),
        ];
        let value = redact_argv(&argv, 500);
        assert!(value.contains("--user-data-dir=<redacted>"));
        assert!(value.contains("--header=<redacted>"));
        assert!(!value.contains("John Doe"));
        assert!(!value.contains("Bearer secret"));
    }

    #[test]
    fn redacts_monolithic_command_line() {
        let argv = vec![
            "chrome --headless --header=Authorization:secret --user-data-dir=/tmp/private"
                .to_string(),
        ];
        let value = redact_argv(&argv, 300);
        assert!(value.contains("--header=<redacted>"));
        assert!(value.contains("--user-data-dir=<redacted>"));
        assert!(!value.contains("Authorization:secret"));
    }
}
