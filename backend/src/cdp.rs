use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::WebSocket;
use url::Url;

use crate::redaction::{redact_text, redact_url};

const MAX_TABS: usize = 16;
const MAX_SCREENSHOT_BYTES: usize = 6_000_000;

#[derive(Clone, Debug, Serialize)]
pub struct CdpTab {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(skip)]
    pub ws_url: String,
}

#[derive(Clone, Debug, Default)]
pub struct CdpProbe {
    pub state: String,
    pub tabs: Vec<CdpTab>,
    pub reason: Option<String>,
}

fn http_get(port: u16, path: &str, timeout: Duration) -> Result<Vec<u8>, String> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).map_err(|err| err.to_string())?;
    stream.set_read_timeout(Some(timeout)).ok();
    stream.set_write_timeout(Some(timeout)).ok();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nAccept: application/json\r\n\r\n"
    )
    .map_err(|err| err.to_string())?;
    let mut data = Vec::new();
    let mut buffer = [0u8; 8192];
    let mut header_end: Option<usize> = None;
    let mut content_length: Option<usize> = None;
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                data.extend_from_slice(&buffer[..count]);
                if header_end.is_none() {
                    if let Some(position) = data.windows(4).position(|window| window == b"\r\n\r\n")
                    {
                        header_end = Some(position + 4);
                        let headers = String::from_utf8_lossy(&data[..position]);
                        for line in headers.lines() {
                            if let Some((name, value)) = line.split_once(':') {
                                if name.eq_ignore_ascii_case("content-length") {
                                    content_length = value.trim().parse::<usize>().ok();
                                }
                            }
                        }
                    }
                }
                if let (Some(start), Some(length)) = (header_end, content_length) {
                    if data.len() >= start + length {
                        break;
                    }
                }
            }
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(err) => return Err(err.to_string()),
        }
    }
    let separator = header_end
        .or_else(|| {
            data.windows(4)
                .position(|window| window == b"\r\n\r\n")
                .map(|value| value + 4)
        })
        .ok_or_else(|| "malformed HTTP response".to_string())?;
    Ok(data[separator..].to_vec())
}

pub fn probe_tabs(port: u16, timeout: Duration) -> CdpProbe {
    if port == 0 {
        return CdpProbe {
            state: "endpoint_unreachable".to_string(),
            tabs: Vec::new(),
            reason: Some("Invalid local debugging port.".to_string()),
        };
    }
    let body = match http_get(port, "/json/list", timeout) {
        Ok(body) => body,
        Err(err) => {
            return CdpProbe {
                state: "endpoint_unreachable".to_string(),
                tabs: Vec::new(),
                reason: Some(format!("Local endpoint did not respond ({err}).")),
            };
        }
    };
    let parsed: Value = match serde_json::from_slice(&body) {
        Ok(value) => value,
        Err(_) => {
            return CdpProbe {
                state: "endpoint_unreachable".to_string(),
                tabs: Vec::new(),
                reason: Some("Endpoint returned an unexpected target list.".to_string()),
            };
        }
    };
    let Some(items) = parsed.as_array() else {
        return CdpProbe {
            state: "endpoint_unreachable".to_string(),
            tabs: Vec::new(),
            reason: Some("Endpoint returned an unexpected target list.".to_string()),
        };
    };
    let mut tabs = Vec::new();
    for item in items {
        if item.get("type").and_then(Value::as_str) != Some("page") {
            continue;
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(100)
            .collect::<String>();
        let title = redact_text(
            item.get("title")
                .and_then(Value::as_str)
                .unwrap_or("Untitled page"),
            240,
        );
        let url = redact_url(item.get("url").and_then(Value::as_str).unwrap_or(""));
        let ws_url = item
            .get("webSocketDebuggerUrl")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        tabs.push(CdpTab {
            id,
            title,
            url,
            ws_url,
        });
        if tabs.len() >= MAX_TABS {
            break;
        }
    }
    CdpProbe {
        state: "available".to_string(),
        tabs,
        reason: None,
    }
}

pub fn local_ws_url(value: &str, expected_port: u16) -> Result<Url, String> {
    let parsed = Url::parse(value).map_err(|_| "Invalid CDP WebSocket URL.".to_string())?;
    if !matches!(parsed.scheme(), "ws" | "wss") {
        return Err("Only local ws/wss CDP endpoints are allowed.".to_string());
    }
    let host = parsed.host_str().unwrap_or_default();
    if !matches!(host, "127.0.0.1" | "localhost" | "::1") {
        return Err("Only local CDP endpoints are allowed.".to_string());
    }
    if parsed.port() != Some(expected_port) {
        return Err("The CDP endpoint does not match the session port.".to_string());
    }
    if !parsed.path().starts_with("/devtools/page/") {
        return Err("Unexpected CDP target path.".to_string());
    }
    Ok(parsed)
}

pub fn capture_screenshot(
    ws_url: &str,
    expected_port: u16,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let _ = local_ws_url(ws_url, expected_port)?;
    let (mut socket, _) =
        tungstenite::connect(ws_url).map_err(|err| format!("CDP connection failed: {err}"))?;
    set_socket_timeout(&mut socket, timeout);
    let request = json!({
        "id": 1,
        "method": "Page.captureScreenshot",
        "params": {"format": "jpeg", "quality": 55, "captureBeyondViewport": false}
    });
    socket
        .send(tungstenite::Message::text(request.to_string()))
        .map_err(|err| err.to_string())?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let message = socket.read().map_err(|err| err.to_string())?;
        let text = match message {
            tungstenite::Message::Text(text) => text.to_string(),
            tungstenite::Message::Binary(bytes) => String::from_utf8_lossy(&bytes).to_string(),
            tungstenite::Message::Close(_) => return Err("CDP closed the connection.".to_string()),
            _ => continue,
        };
        let reply: Value = serde_json::from_str(&text).map_err(|err| err.to_string())?;
        if reply.get("id").and_then(Value::as_i64) != Some(1) {
            continue;
        }
        if let Some(error) = reply.get("error") {
            return Err(format!("CDP screenshot failed: {error}"));
        }
        let encoded = reply
            .pointer("/result/data")
            .and_then(Value::as_str)
            .ok_or_else(|| "CDP returned no image data.".to_string())?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| "CDP returned invalid base64.".to_string())?;
        if bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err("Screenshot is too large to display.".to_string());
        }
        return Ok(bytes);
    }
    Err("Screenshot timed out.".to_string())
}

fn set_socket_timeout(socket: &mut WebSocket<MaybeTlsStream<TcpStream>>, timeout: Duration) {
    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
        stream.set_read_timeout(Some(timeout)).ok();
        stream.set_write_timeout(Some(timeout)).ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_local_or_mismatched_cdp_urls() {
        assert!(local_ws_url("ws://127.0.0.1:9222/devtools/page/x", 9222).is_ok());
        assert!(local_ws_url("ws://evil.example:9222/devtools/page/x", 9222).is_err());
        assert!(local_ws_url("ws://127.0.0.1:9223/devtools/page/x", 9222).is_err());
        assert!(local_ws_url("ws://127.0.0.1:9222/other", 9222).is_err());
    }

    #[test]
    fn redacts_tab_metadata() {
        let tab = CdpTab {
            id: "x".to_string(),
            title: "private".to_string(),
            url: "https://example.com/a?token=secret".to_string(),
            ws_url: "ws://127.0.0.1:1/devtools/page/x".to_string(),
        };
        let serialized = serde_json::to_string(&tab).expect("serialize");
        assert!(!serialized.contains("ws://"));
    }
}
