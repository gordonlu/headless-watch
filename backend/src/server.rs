use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Path, Query, Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::future::join_all;
use futures_util::{stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tower_http::services::ServeDir;
use tracing::debug;

use crate::actions::{terminate, TerminationOutcome, TerminationRequest};
use crate::cdp::capture_screenshot;
use crate::core::Core;
use crate::model::VERSION;
use crate::security::{
    allowed_host, allowed_origin, query_token, safe_content_type, token_matches,
};
use crate::settings::Settings;

const MAX_BODY: usize = 4096;

#[derive(Clone)]
pub struct AppState {
    pub core: Arc<Mutex<Core>>,
    pub tx: broadcast::Sender<String>,
    pub shutdown: tokio::sync::watch::Receiver<bool>,
    pub port: u16,
    pub token: Arc<str>,
    pub index_template: Arc<str>,
    pub web_dir: PathBuf,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({"ok": false, "error": self.message})),
        )
            .into_response()
    }
}

fn secure_headers(response: &mut Response) {
    let csp = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; font-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'; object-src 'none'; media-src 'none'";
    let headers = response.headers_mut();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        "X-Content-Type-Options",
        HeaderValue::from_static("nosniff"),
    );
    headers.insert("Referrer-Policy", HeaderValue::from_static("no-referrer"));
    headers.insert("X-Frame-Options", HeaderValue::from_static("DENY"));
    headers.insert(
        "Cross-Origin-Opener-Policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "Cross-Origin-Resource-Policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        "Permissions-Policy",
        HeaderValue::from_static(
            "camera=(), microphone=(), geolocation=(), usb=(), serial=(), payment=()",
        ),
    );
    headers.insert("Content-Security-Policy", HeaderValue::from_static(csp));
}

fn error_response(status: StatusCode, message: &str) -> Response {
    let mut response = (status, Json(json!({"ok": false, "error": message}))).into_response();
    secure_headers(&mut response);
    response
}

pub fn build_app(state: AppState) -> Router {
    let assets = ServeDir::new(state.web_dir.join("assets"));
    Router::new()
        .route("/", get(index))
        .route("/index.html", get(index))
        .route("/favicon.ico", get(favicon))
        .route("/api/v1/health", get(health))
        .route("/api/v1/overview", get(overview))
        .route("/api/v1/stream", get(stream))
        .route("/api/v1/sessions", get(sessions))
        .route("/api/v1/sessions/{id}", get(session_detail))
        .route("/api/v1/sessions/{id}/metrics", get(session_metrics))
        .route("/api/v1/sessions/{id}/tabs", get(session_tabs))
        .route("/api/v1/sessions/{id}/screenshot", post(screenshot))
        .route("/api/v1/sessions/{id}/terminate", post(terminate_session))
        .route("/api/v1/terminate", post(terminate_batch))
        .route("/api/v1/sessions/{id}/mute", post(mute_session))
        .route("/api/v1/events", get(events))
        .route("/api/v1/actions", get(actions))
        .route("/api/v1/alerts", get(alerts))
        .route("/api/v1/alerts/{id}/ack", post(acknowledge_alert))
        .route("/api/v1/alerts/{id}/mute", post(mute_alert))
        .route("/api/v1/settings", get(settings).patch(update_settings))
        .route("/api/v1/export", get(export))
        .nest_service("/assets", assets)
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            security_middleware,
        ))
        .with_state(state)
}

async fn security_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !allowed_host(host, state.port) {
        return error_response(StatusCode::FORBIDDEN, "Unrecognized Host header.");
    }
    let path = request.uri().path().to_string();
    let method = request.method().clone();
    if path.starts_with("/api/v1/") && path != "/api/v1/health" {
        let header_token = request
            .headers()
            .get("X-Session-Token")
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        let provided = header_token.or_else(|| {
            if path == "/api/v1/stream" {
                query_token(request.uri().query())
            } else {
                None
            }
        });
        if !token_matches(&state.token, provided.as_deref()) {
            return error_response(StatusCode::FORBIDDEN, "Not authorized.");
        }
        if method != Method::GET && method != Method::HEAD {
            let origin = request
                .headers()
                .get(header::ORIGIN)
                .and_then(|value| value.to_str().ok());
            if !allowed_origin(origin, state.port) {
                return error_response(StatusCode::FORBIDDEN, "Cross-origin request rejected.");
            }
            let site = request
                .headers()
                .get("Sec-Fetch-Site")
                .and_then(|value| value.to_str().ok());
            if let Some(site) = site {
                if !matches!(site, "same-origin" | "none") {
                    return error_response(StatusCode::FORBIDDEN, "Cross-site request rejected.");
                }
            }
            let content_type = request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok());
            if content_type.is_some() && !safe_content_type(content_type) {
                return error_response(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "Content-Type must be application/json.",
                );
            }
            let content_length = request
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(0);
            if content_length > 0 && content_type.is_none() {
                return error_response(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "Content-Type must be application/json.",
                );
            }
            if content_length > MAX_BODY {
                return error_response(StatusCode::PAYLOAD_TOO_LARGE, "Request body is too large.");
            }
        }
    }
    let log_method = method.clone();
    let log_path = path.clone();
    let mut response = next.run(request).await;
    debug!(
        target: "headless_watch::http",
        method = %log_method,
        path = %log_path,
        status = response.status().as_u16(),
        "request"
    );
    secure_headers(&mut response);
    response
}

async fn index(State(state): State<AppState>) -> Response {
    let html = state
        .index_template
        .replace("__TOKEN__", state.token.as_ref());
    let mut response = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
    secure_headers(&mut response);
    response
}

async fn favicon() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

async fn health(State(state): State<AppState>) -> Response {
    let core = match state.core.lock() {
        Ok(core) => core,
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "State lock is poisoned.")
        }
    };
    let health = core.tracker.health(Instant::now());
    Json(json!({
        "ok": health.status == "ok",
        "version": VERSION,
        "uptime_s": (crate::util::now_wall() - core.tracker.started_wall).max(0.0) as i64,
        "status": health.status,
        "last_scan_age_s": health.last_scan_age_s,
        "active_sessions": health.active_sessions,
    }))
    .into_response()
}

async fn overview(State(state): State<AppState>) -> Response {
    let core = match state.core.lock() {
        Ok(core) => core,
        Err(_) => {
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "State lock is poisoned.")
        }
    };
    Json(core.snapshot()).into_response()
}

async fn stream(
    State(state): State<AppState>,
) -> axum::response::sse::Sse<
    impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>,
> {
    use axum::response::sse::{Event, KeepAlive, Sse};
    let initial = match state.core.lock() {
        Ok(core) => serde_json::to_string(&core.snapshot()).unwrap_or_else(|_| "{}".to_string()),
        Err(_) => "{\"error\":\"state lock poisoned\"}".to_string(),
    };
    let receiver = state.tx.subscribe();
    let mut shutdown_rx = state.shutdown.clone();
    let shutdown = async move {
        while !*shutdown_rx.borrow() {
            if shutdown_rx.changed().await.is_err() {
                break;
            }
        }
    };
    let initial_stream =
        stream::once(async move { Ok(Event::default().event("state").data(initial)) });
    let live = BroadcastStream::new(receiver).filter_map(|message| async move {
        match message {
            Ok(payload) => {
                let id = serde_json::from_str::<Value>(&payload)
                    .ok()
                    .and_then(|value| value.get("revision").and_then(Value::as_u64))
                    .unwrap_or(0);
                Some(Ok(Event::default()
                    .id(id.to_string())
                    .event("state")
                    .data(payload)))
            }
            Err(_) => None,
        }
    });
    Sse::new(initial_stream.chain(live.take_until(shutdown))).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keepalive"),
    )
}

#[derive(Deserialize, Default)]
struct SessionQuery {
    status: Option<String>,
    sort: Option<String>,
    q: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
}

fn matches_query(value: &str, q: &Option<String>, source: &Option<String>) -> bool {
    if let Some(q) = q {
        if !value.to_ascii_lowercase().contains(&q.to_ascii_lowercase()) {
            return false;
        }
    }
    if let Some(source) = source {
        if !value
            .to_ascii_lowercase()
            .contains(&source.to_ascii_lowercase())
        {
            return false;
        }
    }
    true
}

async fn sessions(
    State(state): State<AppState>,
    Query(query): Query<SessionQuery>,
) -> Result<Response, ApiError> {
    let status = query.status.as_deref().unwrap_or("active");
    let limit = query.limit.unwrap_or(200).min(1000);
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    if status == "ended" {
        let mut rows = core
            .storage
            .list_ended_sessions(limit)
            .map_err(|err| ApiError::internal(err.to_string()))?;
        if let Some(q) = &query.q {
            rows.retain(|row| {
                matches_query(
                    &format!("{} {} {}", row.browser, row.id, row.root_pid),
                    &Some(q.clone()),
                    &None,
                )
            });
        }
        if let Some(source) = &query.source {
            rows.retain(|row| {
                row.source
                    .to_string()
                    .to_ascii_lowercase()
                    .contains(&source.to_ascii_lowercase())
            });
        }
        return Ok(Json(json!({"sessions": rows, "status": "ended"})).into_response());
    }
    let now = crate::util::now_wall();
    let mut views = core.tracker.session_views(&core.settings, now);
    if let Some(q) = &query.q {
        views.retain(|view| {
            matches_query(
                &format!(
                    "{} {} {} {}",
                    view.browser, view.id, view.pid, view.source.summary
                ),
                &Some(q.clone()),
                &None,
            )
        });
    }
    if let Some(source) = &query.source {
        views.retain(|view| {
            view.source
                .summary
                .to_ascii_lowercase()
                .contains(&source.to_ascii_lowercase())
        });
    }
    let sort = query.sort.as_deref().unwrap_or("cpu");
    views.sort_by(|left, right| match sort {
        "memory" => right.memory.rss_sum.cmp(&left.memory.rss_sum),
        "runtime" => right.lifetime_s.cmp(&left.lifetime_s),
        "recent" => right.first_seen_at.total_cmp(&left.first_seen_at),
        "risk" => right.status.risk.cmp(&left.status.risk),
        _ => right
            .cpu
            .cores
            .unwrap_or(0.0)
            .total_cmp(&left.cpu.cores.unwrap_or(0.0)),
    });
    views.truncate(limit);
    Ok(Json(json!({"sessions": views, "status": "active"})).into_response())
}

async fn session_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    if let Some(value) = core
        .tracker
        .session_json(&id, &core.settings, crate::util::now_wall())
    {
        return Ok(Json(value).into_response());
    }
    if let Some(value) = core
        .history_session(&id)
        .map_err(|err| ApiError::internal(err.to_string()))?
    {
        return Ok(Json(value).into_response());
    }
    Err(ApiError::new(StatusCode::NOT_FOUND, "Session not found."))
}

async fn session_metrics(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<Value>,
) -> Result<Response, ApiError> {
    let window_s = parse_window(query.get("window").and_then(Value::as_str)).unwrap_or(1800.0);
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let points = core
        .storage
        .metrics(&id, window_s, 360, core.settings.retention_raw_hours)
        .map_err(|err| ApiError::internal(err.to_string()))?;
    Ok(Json(json!({"session_id": id, "window_s": window_s, "points": points})).into_response())
}

fn parse_window(value: Option<&str>) -> Option<f64> {
    let value = value?;
    let (number, multiplier) = match value.chars().last()? {
        'm' => (&value[..value.len() - 1], 60.0),
        'h' => (&value[..value.len() - 1], 3600.0),
        'd' => (&value[..value.len() - 1], 86400.0),
        _ => (value, 1.0),
    };
    number
        .parse::<f64>()
        .ok()
        .map(|value| (value * multiplier).clamp(60.0, 7.0 * 86400.0))
}

async fn session_tabs(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let Some(value) = core.tracker.tabs_json(&id) else {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Session is not active.",
        ));
    };
    Ok(Json(value).into_response())
}

#[derive(Deserialize)]
struct ScreenshotRequest {
    tab_id: String,
}

async fn screenshot(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<ScreenshotRequest>,
) -> Result<Response, ApiError> {
    let (port, ws_url) = {
        let core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        if !core.settings.allow_screenshots {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "Screenshots are disabled in Settings.",
            ));
        }
        core.tracker
            .screenshot_tab(&id, &request.tab_id)
            .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "CDP tab is not available."))?
    };
    let image = tokio::task::spawn_blocking(move || {
        capture_screenshot(&ws_url, port, Duration::from_secs(4))
    })
    .await
    .map_err(|err| ApiError::internal(err.to_string()))?
    .map_err(|err| ApiError::new(StatusCode::BAD_GATEWAY, err))?;
    let mut response = Response::new(axum::body::Body::from(image));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg"));
    Ok(response)
}

#[derive(Deserialize)]
struct ProcessIdentityBody {
    pid: u32,
    start_ticks: u64,
}

#[derive(Deserialize)]
struct TerminateRequest {
    identity: ProcessIdentityBody,
    mode: String,
    confirm: Option<bool>,
}

async fn reject_action(state: &AppState, id: &str, mode: &str, message: &str) -> ApiError {
    let notify = {
        let Ok(mut core) = state.core.lock() else {
            return ApiError::internal("State lock is poisoned.");
        };
        let notify = !(message.contains("no longer active")
            || message.contains("already terminating")
            || message.contains("Explicit confirmation")
            || message.contains("Mode must be"));
        core.record_rejected_action(id, mode, message, notify)
    };
    if let Some(notify) = notify {
        let _ =
            tokio::task::spawn_blocking(move || crate::notify::send(&notify.title, &notify.body))
                .await;
    }
    ApiError::new(StatusCode::CONFLICT, message)
}

async fn terminate_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<TerminateRequest>,
) -> Result<Response, ApiError> {
    debug!(target: "headless_watch::actions", session = %id, mode = %request.mode, "terminate request received");
    if request.confirm != Some(true) {
        return Err(reject_action(
            &state,
            &id,
            &request.mode,
            "Explicit confirmation is required.",
        )
        .await);
    }
    if request.mode != "term" && request.mode != "kill" {
        return Err(reject_action(&state, &id, "invalid", "Mode must be 'term' or 'kill'.").await);
    }
    let prepared = {
        let mut core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        match core.termination_info(&id) {
            None => Err((
                "term".to_string(),
                "This browser session is no longer active.".to_string(),
            )),
            Some((_, root_pid, root_start, _, _))
                if root_pid != request.identity.pid
                    || root_start != request.identity.start_ticks =>
            {
                Err((
                    "term".to_string(),
                    "Process identity does not match the requested session.".to_string(),
                ))
            }
            Some((_, _, _, _, true)) => Err((
                "term".to_string(),
                "This session is stale or already terminating.".to_string(),
            )),
            Some((boot_id, root_pid, root_start, uid, false)) => {
                core.mark_terminating(&id, &request.mode);
                Ok((
                    core.proc_root.clone(),
                    TerminationRequest {
                        session_id: id.clone(),
                        boot_id,
                        root_pid,
                        root_start_ticks: root_start,
                        session_uid: uid,
                        mode: request.mode.clone(),
                        wait_s: None,
                    },
                ))
            }
        }
    };
    let (proc_root, termination) = match prepared {
        Ok(value) => value,
        Err((mode, message)) => {
            return Err(reject_action(&state, &id, &mode, &message).await);
        }
    };
    debug!(target: "headless_watch::actions", session = %id, "verified; signaling process tree");
    let mode = request.mode;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let task_state = state.clone();
    tokio::spawn(async move {
        let result = run_termination(task_state.clone(), id, mode, termination, proc_root).await;
        let _ = sender.send(result);
        broadcast_state(&task_state);
    });
    match receiver.await {
        Ok(Ok(outcome)) => {
            let status = if outcome.ok {
                StatusCode::OK
            } else {
                StatusCode::CONFLICT
            };
            Ok((status, Json(outcome)).into_response())
        }
        Ok(Err(err)) => Err(err),
        Err(_) => Err(ApiError::internal("Termination task ended unexpectedly.")),
    }
}

#[derive(Deserialize)]
struct BatchTarget {
    id: String,
    identity: ProcessIdentityBody,
}

#[derive(Deserialize)]
struct BatchTerminateRequest {
    sessions: Vec<BatchTarget>,
    mode: String,
    confirm: Option<bool>,
}

async fn terminate_batch(
    State(state): State<AppState>,
    Json(request): Json<BatchTerminateRequest>,
) -> Result<Response, ApiError> {
    if request.confirm != Some(true) {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Explicit confirmation is required.",
        ));
    }
    if request.mode != "term" && request.mode != "kill" {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Mode must be 'term' or 'kill'.",
        ));
    }
    if request.sessions.is_empty() {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Select at least one session.",
        ));
    }
    if request.sessions.len() > 20 {
        return Err(ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Batch cleanup is limited to 20 sessions per request.",
        ));
    }
    let mut prepared = Vec::new();
    let mut rejected = Vec::new();
    {
        let mut core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        for target in request.sessions {
            match core.termination_info(&target.id) {
                None => rejected.push((
                    target.id,
                    "This browser session is no longer active.".to_string(),
                )),
                Some((_, _, _, _, true)) => rejected.push((
                    target.id,
                    "This session is stale or already terminating.".to_string(),
                )),
                Some((_, root_pid, root_start, _, false))
                    if root_pid != target.identity.pid
                        || root_start != target.identity.start_ticks =>
                {
                    rejected.push((
                        target.id,
                        "Process identity does not match the requested session.".to_string(),
                    ));
                }
                Some((boot_id, root_pid, root_start, uid, false)) => {
                    core.mark_terminating(&target.id, &request.mode);
                    let session_id = target.id.clone();
                    prepared.push((
                        session_id.clone(),
                        TerminationRequest {
                            session_id,
                            boot_id,
                            root_pid,
                            root_start_ticks: root_start,
                            session_uid: uid,
                            mode: request.mode.clone(),
                            wait_s: None,
                        },
                    ));
                }
            }
        }
    }
    let mut results = Vec::new();
    for (id, message) in rejected {
        let error = reject_action(&state, &id, &request.mode, &message).await;
        results.push(json!({
            "session_id": id,
            "ok": false,
            "result": "rejected",
            "message": error.message,
        }));
    }
    let proc_root = state
        .core
        .lock()
        .map(|core| core.proc_root.clone())
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let futures = prepared.into_iter().map(|(id, termination)| {
        let state = state.clone();
        let mode = request.mode.clone();
        let proc_root = proc_root.clone();
        async move {
            let session_id = id.clone();
            match run_termination(state, id, mode, termination, proc_root).await {
                Ok(outcome) => json!({
                    "session_id": session_id,
                    "ok": outcome.ok,
                    "result": outcome.result,
                    "message": outcome.message,
                    "targets": outcome.targets,
                    "signaled": outcome.signaled,
                    "survivors": outcome.survivors,
                    "force_available": outcome.force_available,
                }),
                Err(error) => json!({
                    "session_id": session_id,
                    "ok": false,
                    "result": "rejected",
                    "message": error.message,
                }),
            }
        }
    });
    results.extend(join_all(futures).await);
    broadcast_state(&state);
    let ok = results
        .iter()
        .all(|item| item.get("ok").and_then(Value::as_bool) == Some(true));
    Ok(Json(json!({
        "ok": ok,
        "mode": request.mode,
        "count": results.len(),
        "results": results,
    }))
    .into_response())
}

async fn run_termination(
    state: AppState,
    id: String,
    mode: String,
    termination: TerminationRequest,
    proc_root: PathBuf,
) -> Result<TerminationOutcome, ApiError> {
    let outcome = tokio::task::spawn_blocking(move || terminate(&termination, &proc_root))
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?;
    debug!(target: "headless_watch::actions", session = %id, "signal phase completed");
    match outcome {
        Ok(outcome) => {
            debug!(target: "headless_watch::actions", session = %id, result = %outcome.result, "recording action");
            let notify = {
                let mut core = state
                    .core
                    .lock()
                    .map_err(|_| ApiError::internal("State lock is poisoned."))?;
                debug!(target: "headless_watch::actions", session = %id, "core lock acquired for action audit");
                core.finish_termination(&id, &outcome.message, outcome.force_available);
                core.record_action(&id, &mode, &outcome, json!({}))
            };
            debug!(target: "headless_watch::actions", session = %id, "action audit recorded");
            if let Some(notify) = notify {
                let _ = tokio::task::spawn_blocking(move || {
                    crate::notify::send(&notify.title, &notify.body)
                })
                .await;
            }
            debug!(target: "headless_watch::actions", session = %id, "terminate response ready");
            Ok(outcome)
        }
        Err(rejection) => {
            let notify = {
                let mut core = state
                    .core
                    .lock()
                    .map_err(|_| ApiError::internal("State lock is poisoned."))?;
                core.finish_termination(&id, &rejection.0, false);
                core.record_rejected_action(&id, &mode, &rejection.0, true)
            };
            if let Some(notify) = notify {
                let _ = tokio::task::spawn_blocking(move || {
                    crate::notify::send(&notify.title, &notify.body)
                })
                .await;
            }
            Err(ApiError::new(StatusCode::CONFLICT, rejection.0))
        }
    }
}

#[derive(Deserialize)]
struct MuteRequest {
    minutes: Option<u64>,
}

async fn mute_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<MuteRequest>,
) -> Result<Response, ApiError> {
    let changed = {
        let mut core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        core.mute_session(&id, request.minutes.unwrap_or(30))
    };
    if !changed {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "Session is not active.",
        ));
    }
    broadcast_state(&state);
    Ok(Json(json!({"ok": true})).into_response())
}

#[derive(Deserialize)]
struct EventQuery {
    since: Option<f64>,
    limit: Option<usize>,
    session_id: Option<String>,
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventQuery>,
) -> Result<Response, ApiError> {
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let rows = core
        .storage
        .events_since(
            query.since,
            query.limit.unwrap_or(100),
            query.session_id.as_deref(),
        )
        .map_err(|err| ApiError::internal(err.to_string()))?;
    Ok(Json(json!({"events": rows})).into_response())
}

async fn actions(
    State(state): State<AppState>,
    Query(query): Query<Value>,
) -> Result<Response, ApiError> {
    let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let rows = core
        .storage
        .recent_actions(limit)
        .map_err(|err| ApiError::internal(err.to_string()))?;
    Ok(Json(json!({"actions": rows})).into_response())
}

async fn alerts(
    State(state): State<AppState>,
    Query(query): Query<Value>,
) -> Result<Response, ApiError> {
    let limit = query.get("limit").and_then(Value::as_u64).unwrap_or(100) as usize;
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let rows = core
        .alert_list(limit)
        .map_err(|err| ApiError::internal(err.to_string()))?;
    Ok(Json(json!({"alerts": rows})).into_response())
}

async fn acknowledge_alert(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let changed = {
        let mut core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        core.acknowledge_alert(id)
            .map_err(|err| ApiError::internal(err.to_string()))?
    };
    if !changed {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "Alert not found."));
    }
    broadcast_state(&state);
    Ok(Json(json!({"ok": true})).into_response())
}

async fn mute_alert(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(request): Json<MuteRequest>,
) -> Result<Response, ApiError> {
    let changed = {
        let mut core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        core.mute_alert(id, request.minutes.unwrap_or(30))
            .map_err(|err| ApiError::internal(err.to_string()))?
    };
    if !changed {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "Alert not found."));
    }
    broadcast_state(&state);
    Ok(Json(json!({"ok": true})).into_response())
}

async fn settings(State(state): State<AppState>) -> Result<Response, ApiError> {
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    Ok(Json(core.settings_json()).into_response())
}

async fn update_settings(
    State(state): State<AppState>,
    Json(settings): Json<Settings>,
) -> Result<Response, ApiError> {
    {
        let mut core = state
            .core
            .lock()
            .map_err(|_| ApiError::internal("State lock is poisoned."))?;
        core.update_settings(settings)
            .map_err(|err| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, err))?;
    }
    broadcast_state(&state);
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    Ok(Json(core.settings_json()).into_response())
}

async fn export(
    State(state): State<AppState>,
    Query(query): Query<Value>,
) -> Result<Response, ApiError> {
    let format = query
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("json");
    let dataset = query
        .get("dataset")
        .and_then(Value::as_str)
        .unwrap_or("sessions");
    let core = state
        .core
        .lock()
        .map_err(|_| ApiError::internal("State lock is poisoned."))?;
    let payload = core
        .export_json()
        .map_err(|err| ApiError::internal(err.to_string()))?;
    if format == "csv" {
        let rows = payload
            .get(dataset)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let csv = to_csv(&rows);
        let mut response = Response::new(axum::body::Body::from(csv));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/csv; charset=utf-8"),
        );
        response.headers_mut().insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment; filename=\"headless-watch.csv\""),
        );
        return Ok(response);
    }
    Ok(Json(payload).into_response())
}

fn to_csv(rows: &[Value]) -> String {
    let Some(first) = rows.first().and_then(Value::as_object) else {
        return String::new();
    };
    let mut keys = first.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    let mut output = String::new();
    output.push_str(
        &keys
            .iter()
            .map(|key| csv_escape(key))
            .collect::<Vec<_>>()
            .join(","),
    );
    output.push('\n');
    for row in rows {
        let Some(object) = row.as_object() else {
            continue;
        };
        let values = keys
            .iter()
            .map(|key| {
                let value = object.get(key).cloned().unwrap_or(Value::Null);
                match value {
                    Value::Null => String::new(),
                    Value::String(text) => csv_escape(&text),
                    other => csv_escape(&other.to_string()),
                }
            })
            .collect::<Vec<_>>();
        output.push_str(&values.join(","));
        output.push('\n');
    }
    output
}

fn csv_escape(value: &str) -> String {
    if value
        .chars()
        .any(|ch| matches!(ch, ',' | '"' | '\n' | '\r'))
    {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

async fn not_found() -> Response {
    error_response(StatusCode::NOT_FOUND, "Not found.")
}

pub fn broadcast_state(state: &AppState) {
    if state.tx.receiver_count() == 0 {
        return;
    }
    let payload = {
        let Ok(core) = state.core.lock() else {
            return;
        };
        serde_json::to_string(&core.snapshot()).unwrap_or_else(|_| "{}".to_string())
    };
    let _ = state.tx.send(payload);
}

pub fn read_index_template(web_dir: &std::path::Path) -> anyhow::Result<String> {
    Ok(std::fs::read_to_string(web_dir.join("index.html"))?)
}
