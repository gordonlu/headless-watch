use url::{form_urlencoded, Url};

pub fn allowed_host(host: Option<&str>, port: u16) -> bool {
    let Some(host) = host else {
        return false;
    };
    if !host
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':'))
    {
        return false;
    }
    let host = host.to_ascii_lowercase();
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

pub fn allowed_origin(origin: Option<&str>, port: u16) -> bool {
    let Some(origin) = origin else {
        return true;
    };
    let Ok(parsed) = Url::parse(origin) else {
        return false;
    };
    parsed.scheme() == "http"
        && matches!(parsed.host_str(), Some("127.0.0.1") | Some("localhost"))
        && parsed.port() == Some(port)
}

pub fn token_matches(expected: &str, provided: Option<&str>) -> bool {
    let Some(provided) = provided else {
        return false;
    };
    let left = expected.as_bytes();
    let right = provided.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

pub fn query_token(query: Option<&str>) -> Option<String> {
    let query = query?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == "token")
        .map(|(_, value)| value.to_string())
}

pub fn safe_content_type(value: Option<&str>) -> bool {
    value
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_hosts_and_origins() {
        assert!(allowed_host(Some("127.0.0.1:8765"), 8765));
        assert!(allowed_host(Some("localhost:8765"), 8765));
        assert!(!allowed_host(Some("evil.example:8765"), 8765));
        assert!(!allowed_host(Some("127.0.0.1:9999"), 8765));
        assert!(allowed_origin(None, 8765));
        assert!(allowed_origin(Some("http://127.0.0.1:8765"), 8765));
        assert!(!allowed_origin(Some("http://evil.example:8765"), 8765));
        assert!(!allowed_origin(Some("https://127.0.0.1:8765"), 8765));
    }

    #[test]
    fn compares_tokens_and_content_type() {
        assert!(token_matches("abc", Some("abc")));
        assert!(!token_matches("abc", Some("abd")));
        assert!(!token_matches("abc", None));
        assert!(safe_content_type(Some("application/json; charset=utf-8")));
        assert!(!safe_content_type(Some("text/plain")));
    }
}
