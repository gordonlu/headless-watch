use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{header, HeaderMap, Method, Request, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast;
use tower::ServiceExt;

use headless_watch::core::Core;
use headless_watch::server::{build_app, read_index_template, AppState};

struct Harness {
    app: Router,
    token: String,
    _data: TempDir,
    _web: TempDir,
}

fn build_harness() -> Harness {
    let data = tempfile::tempdir().expect("data tempdir");
    let web = tempfile::tempdir().expect("web tempdir");
    std::fs::write(
        web.path().join("index.html"),
        "<!doctype html><meta name=\"hw-token\" content=\"__TOKEN__\"><div id=\"app\"></div>",
    )
    .expect("write index");
    std::fs::create_dir_all(web.path().join("assets")).expect("assets dir");
    let core = Core::open(data.path(), web.path().to_path_buf(), None, None).expect("core");
    let token = core.token.clone();
    let index = read_index_template(web.path()).expect("index template");
    let (tx, _) = broadcast::channel(8);
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let state = AppState {
        core: Arc::new(Mutex::new(core)),
        tx,
        shutdown: shutdown_rx,
        port: 8765,
        token: Arc::<str>::from(token.clone()),
        index_template: Arc::<str>::from(index),
        web_dir: web.path().to_path_buf(),
    };
    Harness {
        app: build_app(state),
        token,
        _data: data,
        _web: web,
    }
}

fn request(
    method: Method,
    uri: &str,
    token: Option<&str>,
    body: Option<Value>,
    origin: Option<&str>,
    host: &str,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, host);
    if let Some(token) = token {
        builder = builder.header("X-Session-Token", token);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    let data = match body {
        Some(value) => {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
            serde_json::to_vec(&value).expect("serialize body")
        }
        None => Vec::new(),
    };
    builder.body(Body::from(data)).expect("request")
}

async fn send(app: &Router, request: Request<Body>) -> (StatusCode, HeaderMap, Value) {
    let response = app.clone().oneshot(request).await.expect("response");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).to_string()))
    };
    (status, headers, value)
}

fn host_header() -> &'static str {
    "127.0.0.1:8765"
}

#[tokio::test]
async fn serves_index_with_token_and_security_headers() {
    let harness = build_harness();
    let (status, headers, body) = send(
        &harness.app,
        request(Method::GET, "/", None, None, None, host_header()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let text = body.as_str().expect("html body");
    assert!(text.contains(&harness.token));
    assert!(!text.contains("__TOKEN__"));
    let csp = headers
        .get("Content-Security-Policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(csp.contains("script-src 'self'"));
    assert!(!csp.contains("unsafe-inline"));
    assert_eq!(
        headers
            .get("X-Frame-Options")
            .and_then(|value| value.to_str().ok()),
        Some("DENY")
    );
}

#[tokio::test]
async fn validates_host_token_origin_and_content_type() {
    let harness = build_harness();
    let (status, _, _) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/overview",
            Some(&harness.token),
            None,
            None,
            "evil.example",
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, _) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/overview",
            None,
            None,
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/overview",
            Some(&harness.token),
            None,
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(payload.get("health").is_some());
    assert!(payload.get("summary").is_some());

    let (status, _, health) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/health",
            None,
            None,
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(health
        .get("monitor_pid")
        .and_then(serde_json::Value::as_u64)
        .is_some());

    let (status, _, _) = send(
        &harness.app,
        request(
            Method::POST,
            "/api/v1/sessions/unknown/terminate",
            Some(&harness.token),
            Some(json!({"identity":{"pid":1,"start_ticks":1},"mode":"term","confirm":true})),
            Some("http://evil.example:8765"),
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let mut bad_type = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/sessions/unknown/terminate")
        .header(header::HOST, host_header())
        .header("X-Session-Token", &harness.token)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("{}"))
        .expect("request");
    let (status, _, _) = send(&harness.app, bad_type).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    bad_type = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/stream")
        .header(header::HOST, host_header())
        .body(Body::empty())
        .expect("request");
    let (status, _, _) = send(&harness.app, bad_type).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn patches_settings_and_rejects_invalid_ranges() {
    let harness = build_harness();
    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/settings",
            Some(&harness.token),
            None,
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let mut settings = payload.get("settings").cloned().expect("settings");
    settings["interval_s"] = json!(2.0);
    settings["notifications"] = json!(false);
    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::PATCH,
            "/api/v1/settings",
            Some(&harness.token),
            Some(settings.clone()),
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["settings"]["interval_s"], json!(2.0));
    assert_eq!(payload["settings"]["notifications"], json!(false));

    settings["interval_s"] = json!(0.1);
    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::PATCH,
            "/api/v1/settings",
            Some(&harness.token),
            Some(settings),
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(payload.get("error").is_some());
}

#[tokio::test]
async fn validates_batch_termination_requests() {
    let harness = build_harness();
    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::POST,
            "/api/v1/terminate",
            Some(&harness.token),
            Some(json!({"sessions": [], "mode": "term", "confirm": true})),
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(payload.get("error").is_some());

    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::POST,
            "/api/v1/terminate",
            Some(&harness.token),
            Some(json!({
                "sessions": [{"id": "s-missing", "identity": {"pid": 1, "start_ticks": 1}}],
                "mode": "term",
                "confirm": true
            })),
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(payload["ok"], json!(false));
    assert_eq!(payload["results"][0]["result"], json!("rejected"));
}

#[tokio::test]
async fn exports_json_and_csv() {
    let harness = build_harness();
    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/export?format=json",
            Some(&harness.token),
            None,
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(payload.get("sessions").is_some());
    assert!(payload.get("events").is_some());

    let (status, headers, _) = send(
        &harness.app,
        request(
            Method::GET,
            "/api/v1/export?format=csv&dataset=events",
            Some(&harness.token),
            None,
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .starts_with("text/csv"));
}

#[tokio::test]
async fn protects_screenshot_and_unknown_sessions() {
    let harness = build_harness();
    let (status, _, _) = send(
        &harness.app,
        request(
            Method::POST,
            "/api/v1/sessions/unknown/screenshot",
            None,
            Some(json!({"tab_id":"x"})),
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, _, payload) = send(
        &harness.app,
        request(
            Method::POST,
            "/api/v1/sessions/unknown/screenshot",
            Some(&harness.token),
            Some(json!({"tab_id":"x"})),
            None,
            host_header(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(payload["error"]
        .as_str()
        .unwrap_or_default()
        .contains("CDP tab"));
}
