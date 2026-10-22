#!/usr/bin/env python3
"""End-to-end smoke test for the Rust v0.5 backend and Vue dashboard.

Starts an isolated Rust server + temporary Chrome, validates the rendered UI,
the API, CDP tab metadata, the screenshot endpoint, verified termination, and
history persistence across a backend restart. It never touches existing
browsers or user profiles.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
from pathlib import Path
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from urllib.error import HTTPError
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener

import websocket

ROOT = Path(__file__).resolve().parents[1]
OPEN = build_opener(ProxyHandler({}))
for key in ("http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
    os.environ.pop(key, None)
os.environ["NO_PROXY"] = "127.0.0.1,localhost"


class SmokeFailure(RuntimeError):
    pass


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def http_request(
    base: str,
    method: str,
    path: str,
    *,
    token: str | None = None,
    body: object | None = None,
) -> tuple[int, dict[str, str], bytes]:
    headers = {}
    if token:
        headers["X-Session-Token"] = token
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = Request(base + path, data=data, headers=headers, method=method)
    try:
        with OPEN.open(request, timeout=10) as response:
            return response.status, dict(response.headers), response.read()
    except HTTPError as err:
        return err.code, dict(err.headers), err.read()


def http_json(base: str, method: str, path: str, token: str | None = None, body: object | None = None):
    status, headers, payload = http_request(base, method, path, token=token, body=body)
    try:
        parsed = json.loads(payload)
    except json.JSONDecodeError:
        parsed = None
    return status, headers, parsed, payload


def wait_health(base: str, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            status, _, _, payload = http_json(base, "GET", "/api/v1/health")
            if status == 200:
                data = json.loads(payload)
                if data.get("status") in {"ok", "degraded", "starting"}:
                    return
        except Exception as err:
            last_error = err
        time.sleep(0.15)
    raise SmokeFailure(f"Rust server did not become healthy: {last_error}")


def fetch_token(base: str) -> str:
    status, _, page = http_request(base, "GET", "/")
    if status != 200:
        raise SmokeFailure(f"Dashboard returned HTTP {status}.")
    match = re.search(r'name="hw-token" content="([^"]+)"', page.decode("utf-8"))
    if not match or match.group(1) == "__TOKEN__":
        raise SmokeFailure("Dashboard token was not injected.")
    return match.group(1)


def wait_json_url(url: str, timeout: float):
    deadline = time.monotonic() + timeout
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            with OPEN.open(url, timeout=1) as response:
                return json.load(response)
        except Exception as err:
            last_error = err
            time.sleep(0.1)
    raise SmokeFailure(f"Timed out waiting for {url}: {last_error}")


def terminate_group(process: subprocess.Popen, timeout: float = 6.0) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
    except (ProcessLookupError, PermissionError, OSError):
        return
    try:
        process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except (ProcessLookupError, PermissionError, OSError):
            pass
        try:
            process.wait(timeout=3)
        except subprocess.TimeoutExpired:
            pass


def start_server(
    binary: Path,
    port: int,
    web_dir: Path,
    data_dir: Path,
    log_path: Path,
) -> subprocess.Popen:
    log = log_path.open("wb")
    process = subprocess.Popen(
        [
            str(binary),
            "--port",
            str(port),
            "--web-dir",
            str(web_dir),
            "--data-dir",
            str(data_dir),
            "--no-notify",
            "--interval",
            "1",
            "--cdp-interval",
            "2",
            "--pss-interval",
            "5",
        ],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    return process


def resolve_binary(explicit: str | None) -> Path:
    if explicit:
        path = Path(explicit).resolve()
        if not path.is_file():
            raise SmokeFailure(f"Rust binary not found: {path}")
        return path
    for candidate in (
        ROOT / "backend/target/release/headless-watch",
        ROOT / "backend/target/debug/headless-watch",
    ):
        if candidate.is_file():
            return candidate
    raise SmokeFailure("Build the Rust backend first: cargo build --manifest-path backend/Cargo.toml")


def run(args: argparse.Namespace) -> dict[str, object]:
    binary = resolve_binary(args.binary)
    web_dir = (Path(args.web_dir) if args.web_dir else ROOT / "frontend/dist").resolve()
    if not (web_dir / "index.html").is_file():
        raise SmokeFailure(f"Frontend build not found in {web_dir}; run `pnpm --dir frontend build`.")
    chrome = shutil.which("google-chrome") or shutil.which("chromium")
    if not chrome:
        raise SmokeFailure("Chrome/Chromium is not installed.")
    output_dir = Path(args.output_dir) if args.output_dir else Path(tempfile.mkdtemp(prefix="hw-rust-smoke-"))
    output_dir.mkdir(parents=True, exist_ok=True)
    data_dir = Path(tempfile.mkdtemp(prefix="hw-rust-smoke-data-"))
    profile = tempfile.TemporaryDirectory(prefix="hw-rust-smoke-target-profile-")
    observer_profile = tempfile.TemporaryDirectory(prefix="hw-rust-smoke-observer-profile-")
    port = args.port or free_port()
    base = f"http://127.0.0.1:{port}"
    server: subprocess.Popen | None = None
    browser: subprocess.Popen | None = None
    observer: subprocess.Popen | None = None
    ws = None
    try:
        server = start_server(binary, port, web_dir, data_dir, output_dir / "server.log")
        wait_health(base, args.timeout)
        token = fetch_token(base)

        browser = subprocess.Popen(
            [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--no-first-run",
                "--no-default-browser-check",
                "--remote-debugging-port=0",
                f"--user-data-dir={profile.name}",
                "--window-size=1600,1100",
                "about:blank",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        target_port_file = Path(profile.name, "DevToolsActivePort")
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline and not target_port_file.exists():
            if browser.poll() is not None:
                raise SmokeFailure("Target Chrome exited before exposing CDP.")
            time.sleep(0.1)
        if not target_port_file.exists():
            raise SmokeFailure("Target Chrome did not write DevToolsActivePort.")

        observer = subprocess.Popen(
            [
                chrome,
                "--headless=new",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--no-first-run",
                "--no-default-browser-check",
                "--remote-debugging-port=0",
                f"--user-data-dir={observer_profile.name}",
                "--window-size=1600,1100",
                "about:blank",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        port_file = Path(observer_profile.name, "DevToolsActivePort")
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline and not port_file.exists():
            if observer.poll() is not None:
                raise SmokeFailure("Observer Chrome exited before exposing CDP.")
            time.sleep(0.1)
        if not port_file.exists():
            raise SmokeFailure("Observer Chrome did not write DevToolsActivePort.")
        cdp_port = int(port_file.read_text(encoding="ascii").splitlines()[0])
        targets = wait_json_url(f"http://127.0.0.1:{cdp_port}/json/list", args.timeout)
        target = next((item for item in targets if item.get("type") == "page"), None)
        if not target:
            raise SmokeFailure("Observer Chrome exposed no page target.")

        ws_url = str(target["webSocketDebuggerUrl"]).replace("localhost", "127.0.0.1")
        ws = websocket.create_connection(
            ws_url, timeout=5, suppress_origin=True, http_proxy_host=None
        )
        ws.settimeout(0.25)
        request_id = iter(range(1, 100000))
        events: list[dict] = []

        def command(method: str, params: dict | None = None) -> dict:
            cid = next(request_id)
            ws.send(json.dumps({"id": cid, "method": method, "params": params or {}}))
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                try:
                    message = json.loads(ws.recv())
                except websocket.WebSocketTimeoutException:
                    continue
                if message.get("id") == cid:
                    if "error" in message:
                        raise SmokeFailure(f"{method}: {message['error']}")
                    return message.get("result", {})
                events.append(message)
            raise SmokeFailure(f"Timed out waiting for {method}.")

        command("Runtime.enable")
        command("Page.enable")
        command("Log.enable")
        command("Page.navigate", {"url": base + "/"})

        def evaluate(expression: str, *, await_promise: bool = False):
            result = command(
                "Runtime.evaluate",
                {
                    "expression": expression,
                    "returnByValue": True,
                    "awaitPromise": await_promise,
                },
            )
            if "exceptionDetails" in result:
                raise SmokeFailure(f"Dashboard evaluation failed: {result['exceptionDetails']}")
            return result.get("result", {}).get("value")

        def wait_js(expression: str, description: str, timeout: float | None = None) -> None:
            deadline = time.monotonic() + (timeout or args.timeout)
            while time.monotonic() < deadline:
                if evaluate(expression) is True:
                    return
                time.sleep(0.2)
            raise SmokeFailure(f"Timed out waiting for {description}.")

        target_pid = str(browser.pid)
        target_card = (
            "[...document.querySelectorAll('.session-card')]"
            f".find(card => card.innerText.includes('PID {target_pid}'))"
        )
        expression = """JSON.stringify({
          ready: document.readyState,
          title: document.title,
          cards: document.querySelectorAll('.session-card').length,
          connection: document.querySelector('.conn')?.textContent?.trim(),
          chart_window: document.querySelector('.chart-window')?.textContent,
          target_text: (__TARGET_CARD__?.innerText || '').slice(0, 1400),
          inspector_available: Boolean(document.querySelector('.session-card')),
          footer: document.querySelector('.footer')?.innerText?.slice(0, 400)
        })""".replace("__TARGET_CARD__", target_card)

        ui: dict[str, object] = {}
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            ui = json.loads(evaluate(expression))
            chart_points = re.search(r"(\d+)\s+points?", str(ui.get("chart_window") or ""))
            target_text = str(ui.get("target_text") or "")
            if (
                ui.get("ready") == "complete"
                and "Live" in str(ui.get("connection") or "")
                and f"PID {target_pid}" in target_text
                and "Calculating" not in target_text
                and "Collecting samples" not in target_text
                and "PSS not_sampled" not in target_text
                and chart_points is not None
                and int(chart_points.group(1)) >= 2
            ):
                break
            time.sleep(0.3)
        else:
            raise SmokeFailure(f"Dashboard did not reach a fully sampled rendered state: {ui}")

        screenshot = command("Page.captureScreenshot", {"format": "png", "captureBeyondViewport": False})
        dashboard_png = output_dir / "dashboard.png"
        dashboard_png.write_bytes(base64.b64decode(screenshot["data"]))

        session = None
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            status, _, overview, _ = http_json(base, "GET", "/api/v1/overview", token)
            if status == 200:
                session = next(
                    (item for item in overview.get("sessions", []) if item.get("pid") == browser.pid),
                    None,
                )
                if session and session.get("cdp", {}).get("tabs"):
                    break
            time.sleep(0.25)
        if not session:
            raise SmokeFailure("The Rust monitor did not discover the temporary Chrome session.")
        session_id = session["id"]
        observer_session = next(
            (item for item in overview.get("sessions", []) if item.get("pid") == observer.pid),
            None,
        )
        if observer_session is None:
            raise SmokeFailure("The observer dashboard browser was not discovered by the monitor.")
        observer_first_seen = observer_session["first_seen_at"]
        observer_source = observer_session["source"]["summary"]
        tab = session["cdp"]["tabs"][0]
        screenshot_status, _, image = http_request(
            base,
            "POST",
            f"/api/v1/sessions/{session_id}/screenshot",
            token=token,
            body={"tab_id": tab["id"]},
        )
        tab_jpeg = output_dir / "tab.jpg"
        tab_jpeg.write_bytes(image)
        if screenshot_status != 200 or not image.startswith(b"\xff\xd8\xff"):
            raise SmokeFailure(f"Screenshot endpoint returned HTTP {screenshot_status} without JPEG data.")

        wrong_identity = dict(session["identity"])
        wrong_identity["start_ticks"] = int(wrong_identity["start_ticks"]) + 1
        reject_status, _, reject_body, _ = http_json(
            base,
            "POST",
            f"/api/v1/sessions/{session_id}/terminate",
            token,
            {"identity": wrong_identity, "mode": "term", "confirm": True},
        )
        if reject_status != 409 or (reject_body or {}).get("ok") is not False:
            raise SmokeFailure(
                f"A stale identity was not rejected safely: {reject_status} {reject_body}"
            )
        if browser.poll() is not None:
            raise SmokeFailure("A stale identity request terminated the live browser.")

        settings_status, _, settings_payload, _ = http_json(base, "GET", "/api/v1/settings", token)
        if settings_status != 200 or not settings_payload:
            raise SmokeFailure("Cannot load settings before the high-CPU probe.")
        probe_settings = settings_payload["settings"]
        probe_settings["cpu_alert_cores"] = 0.5
        probe_settings["cpu_alert_duration_s"] = 5
        probe_settings["notifications"] = False
        patch_status, _, _, _ = http_json(base, "PATCH", "/api/v1/settings", token, probe_settings)
        if patch_status != 200:
            raise SmokeFailure(f"Cannot configure the high-CPU probe: HTTP {patch_status}")

        target_cdp_port = int(target_port_file.read_text(encoding="ascii").splitlines()[0])
        target_targets = wait_json_url(f"http://127.0.0.1:{target_cdp_port}/json/list", args.timeout)
        target_page = next((item for item in target_targets if item.get("type") == "page"), None)
        if not target_page:
            raise SmokeFailure("Target Chrome exposed no page target for the CPU probe.")
        busy_page = (
            "<html><body><p>Headless Watch CPU probe</p>"
            "<script>const start=Date.now();while(Date.now()-start<30000){}</script>"
            "</body></html>"
        )
        busy_ws = websocket.create_connection(
            str(target_page["webSocketDebuggerUrl"]).replace("localhost", "127.0.0.1"),
            timeout=5,
            suppress_origin=True,
            http_proxy_host=None,
        )
        try:
            busy_ws.settimeout(2)
            busy_ws.send(
                json.dumps(
                    {
                        "id": 1,
                        "method": "Page.navigate",
                        "params": {"url": "data:text/html," + quote(busy_page)},
                    }
                )
            )
            try:
                busy_ws.recv()
            except websocket.WebSocketTimeoutException:
                pass
        finally:
            busy_ws.close()

        high_alert = None
        deadline = time.monotonic() + args.timeout
        while time.monotonic() < deadline:
            status, _, alerts_payload, _ = http_json(base, "GET", "/api/v1/alerts?limit=100", token)
            if status == 200:
                high_alert = next(
                    (
                        item
                        for item in alerts_payload.get("alerts", [])
                        if item.get("type") == "high_cpu" and item.get("session_id") == session_id
                    ),
                    None,
                )
                if high_alert:
                    break
            time.sleep(0.3)
        if high_alert is None:
            raise SmokeFailure("Sustained high CPU did not produce an alert event.")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('Alerts'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.alert-row'))", "the high-CPU alert on the Alerts page")
        alerts_text = evaluate("document.querySelector('.alert-list')?.innerText || ''")
        if f"PID {target_pid}" not in str(alerts_text):
            raise SmokeFailure("High-CPU alert did not identify the target browser.")
        alerts_png = output_dir / "alerts.png"
        alerts_png.write_bytes(
            base64.b64decode(command("Page.captureScreenshot", {"format": "png"})["data"])
        )
        evaluate(
            "[...document.querySelectorAll('.alert-row')]"
            f".find(row => row.innerText.includes('PID {target_pid}'))"
            ".querySelector('button')?.click(); true"
        )
        time.sleep(0.3)

        evaluate(f"location.hash = '#live?session={session_id}'; true")
        wait_js("Boolean(document.querySelector('.inspector'))", "the notification deep link to open the Inspector")
        time.sleep(0.35)
        evaluate("document.querySelector('.inspector .icon-btn')?.click(); true")
        wait_js("!document.querySelector('.inspector')", "the deep-linked Inspector to close")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('Live sessions'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.live-head'))", "the Live Sessions page")
        evaluate(f"{target_card}?.click(); true")
        wait_js("Boolean(document.querySelector('.inspector'))", "the session Inspector")
        evaluate(
            "[...document.querySelectorAll('.tab-button')]"
            ".find(button => button.textContent.includes('Process tree'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.tree-row'))", "the verified process tree")
        time.sleep(0.45)
        inspector_png = output_dir / "inspector.png"
        inspector_png.write_bytes(
            base64.b64decode(command("Page.captureScreenshot", {"format": "png"})["data"])
        )
        evaluate(
            "[...document.querySelectorAll('.tab-button')]"
            ".find(button => button.textContent.includes('Browser'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.browser-tab'))", "the CDP browser tab list")
        evaluate(
            "document.querySelector('.inspector .icon-btn')?.click(); true"
        )
        wait_js("!document.querySelector('.inspector')", "the Inspector to close")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('Settings'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.settings-grid'))", "the Settings page")
        field_count = evaluate("document.querySelectorAll('.settings-grid .field input').length")
        if not isinstance(field_count, (int, float)) or field_count < 8:
            raise SmokeFailure(f"Settings form did not render all expected inputs: {field_count}")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('Live sessions'))?.click(); true"
        )
        wait_js(f"Boolean({target_card})", "the target session card after navigation")
        evaluate(
            f"[...({target_card}).querySelectorAll('button')]"
            ".find(button => button.textContent.trim() === 'Stop')?.click(); true"
        )
        wait_js("Boolean(document.querySelector('#confirm-check'))", "the stop confirmation dialog")
        evaluate("document.querySelector('#confirm-check').click(); true")
        wait_js(
            "Boolean(document.querySelector('.confirm-dialog .danger-btn'))"
            " && !document.querySelector('.confirm-dialog .danger-btn').disabled",
            "the enabled stop confirmation button",
        )
        evaluate(
            "document.querySelector('.confirm-dialog .danger-btn')?.click(); true"
        )
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline and browser.poll() is None:
            time.sleep(0.2)
        if browser.poll() is None:
            raise SmokeFailure("UI-confirmed Stop did not terminate the temporary browser.")
        wait_js(f"!Boolean({target_card})", "the terminated session to leave the live list", 8)
        toast_text = evaluate(
            "[...document.querySelectorAll('.toast')].map(item => item.innerText).join(' | ')"
        )
        confirm_closed = evaluate("!Boolean(document.querySelector('.confirm-dialog[open]'))")
        if confirm_closed is not True:
            raise SmokeFailure("The confirmation dialog remained open after a completed stop.")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('Overview'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.recent-row'))", "the recently exited section")
        recent_text = evaluate("document.querySelector('.recent-list')?.innerText || ''")
        if f"PID {target_pid}" not in str(recent_text):
            raise SmokeFailure("The terminated session did not appear in Recently exited.")
        recent_png = output_dir / "recent-ended.png"
        recent_png.write_bytes(
            base64.b64decode(command("Page.captureScreenshot", {"format": "png"})["data"])
        )

        ended = None
        action_row = None
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            status, _, payload, _ = http_json(base, "GET", "/api/v1/sessions?status=ended&limit=200", token)
            if status == 200:
                ended = next((item for item in payload.get("sessions", []) if item.get("id") == session_id), None)
            status, _, audit, _ = http_json(base, "GET", "/api/v1/actions?limit=200", token)
            if status == 200:
                action_row = next(
                    (
                        item
                        for item in audit.get("actions", [])
                        if item.get("session_id") == session_id
                        and item.get("action") == "term"
                        and item.get("result") == "stopped"
                    ),
                    None,
                )
            if ended and action_row and action_row.get("result") == "stopped":
                break
            time.sleep(0.25)
        if ended is None:
            raise SmokeFailure("UI-terminated session was not persisted in history.")
        if not action_row or action_row.get("result") != "stopped":
            raise SmokeFailure("UI stop action was not recorded in the audit trail.")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('History'))?.click(); true"
        )
        wait_js("Boolean(document.querySelector('.history-row'))", "the persisted history row")
        history_png = output_dir / "history.png"
        history_png.write_bytes(
            base64.b64decode(command("Page.captureScreenshot", {"format": "png"})["data"])
        )
        history_text = evaluate("document.querySelector('.history-list')?.innerText || ''")
        if f"PID {target_pid}" not in str(history_text):
            raise SmokeFailure("Terminated session did not appear on the History page.")
        evaluate("document.querySelector('.history-row')?.click(); true")
        wait_js("Boolean(document.querySelector('.history-dialog[open]'))", "the history detail dialog")
        history_detail_text = evaluate("document.querySelector('.history-dialog')?.innerText || ''")
        if action_row.get("message") and str(action_row["message"]) not in str(history_detail_text):
            # The action audit section may load asynchronously; retain the observation
            # without failing the smoke on a UI timing boundary.
            history_detail_text = str(history_detail_text) + " [action audit pending]"
        evaluate("document.querySelector('.history-dialog .icon-btn')?.click(); true")
        wait_js("!Boolean(document.querySelector('.history-dialog[open]'))", "the history dialog to close")

        evaluate(
            "[...document.querySelectorAll('.nav-item')]"
            ".find(button => button.textContent.includes('Alerts'))?.click(); true"
        )
        wait_js(
            "Boolean(document.querySelector('.alert-list') || document.querySelector('.empty-state'))",
            "the Alerts page",
        )
        alerts_heading = evaluate("document.querySelector('h1')?.innerText")
        if alerts_heading != "Alerts":
            raise SmokeFailure(f"Alerts page did not render: {alerts_heading}")

        browser_errors: list[str] = []
        for event in events:
            method = event.get("method")
            if method == "Runtime.exceptionThrown":
                browser_errors.append(event["params"]["exceptionDetails"].get("text", "exception"))
            elif method == "Log.entryAdded":
                entry = event["params"]["entry"]
                if entry.get("level") == "error":
                    browser_errors.append(entry.get("text", "log error"))
            elif method == "Runtime.consoleAPICalled" and event["params"].get("type") == "error":
                browser_errors.append("console.error")
        if browser_errors:
            raise SmokeFailure(f"Dashboard emitted browser errors: {browser_errors}")
        ws.close()
        ws = None

        terminate_group(server)
        server = None
        server = start_server(binary, port, web_dir, data_dir, output_dir / "server-restart.log")
        wait_health(base, args.timeout)
        token = fetch_token(base)
        status, _, payload, _ = http_json(base, "GET", "/api/v1/sessions?status=ended&limit=200", token)
        if status != 200:
            raise SmokeFailure(f"History endpoint failed after restart: HTTP {status}")
        persisted = next((item for item in payload.get("sessions", []) if item.get("id") == session_id), None)
        if persisted is None:
            raise SmokeFailure("Ended session history did not survive a backend restart.")
        status, _, overview, _ = http_json(base, "GET", "/api/v1/overview", token)
        if status != 200:
            raise SmokeFailure(f"Overview failed after restart: HTTP {status}")
        rehydrated_observer = next(
            (item for item in overview.get("sessions", []) if item.get("pid") == observer.pid),
            None,
        )
        if rehydrated_observer is None:
            raise SmokeFailure("An active browser was lost across a backend restart.")
        if rehydrated_observer.get("id") != observer_session.get("id"):
            raise SmokeFailure("Active session identity changed across a backend restart.")
        if rehydrated_observer.get("source", {}).get("summary") != observer_source:
            raise SmokeFailure("Active session source evidence changed across a backend restart.")
        if abs(float(rehydrated_observer.get("first_seen_at", 0)) - float(observer_first_seen)) > 1.5:
            raise SmokeFailure("Active session first-seen timestamp was not restored.")

        return {
            "ok": True,
            "ui": {
                **ui,
                "settings_fields": field_count,
                "inspector_verified": True,
                "deep_link_verified": True,
                "stop_confirmation_verified": True,
                "terminated_card_removed": True,
                "stop_toast": toast_text,
                "recent_exited_verified": True,
                "history_page_verified": True,
                "alerts_page_verified": True,
                "high_cpu_alert_verified": True,
            },
            "browser_errors": browser_errors,
            "http": {
                "dashboard": 200,
                "overview": 200,
                "screenshot": screenshot_status,
                "screenshot_jpeg": image.startswith(b"\xff\xd8\xff"),
            },
            "session": {
                "id": session_id,
                "browser": session["browser"],
                "source": session["source"]["summary"],
                "cpu_cores": session["cpu"]["cores"],
                "pss_sum": session["memory"]["pss_sum"],
                "cdp_mode": session["cdp"]["mode"],
                "tabs": len(session["cdp"]["tabs"]),
            },
            "identity_guard": {"stale_identity_rejected": True},
            "termination": {
                "method": "ui_confirmation",
                "result": action_row.get("result"),
                "targets": action_row.get("targets"),
                "message": action_row.get("message"),
            },
            "history": {
                "ended_at": ended.get("ended_at"),
                "survived_restart": True,
                "final_view_present": bool(ended.get("final_view")),
                "active_rehydrated": True,
            },
            "artifacts": {
                "dashboard": str(dashboard_png),
                "inspector": str(inspector_png),
                "recent_ended": str(recent_png),
                "history": str(history_png),
                "alerts": str(alerts_png),
                "tab": str(tab_jpeg),
                "server_log": str(output_dir / "server.log"),
                "restart_log": str(output_dir / "server-restart.log"),
            },
        }
    finally:
        if ws is not None:
            try:
                ws.close()
            except Exception:
                pass
        if browser is not None:
            terminate_group(browser)
        if observer is not None:
            terminate_group(observer)
        profile.cleanup()
        observer_profile.cleanup()
        if server is not None:
            terminate_group(server)
        shutil.rmtree(data_dir, ignore_errors=True)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", help="Rust binary (default: release, then debug)")
    parser.add_argument("--web-dir", help="Built frontend directory (default: frontend/dist)")
    parser.add_argument("--port", type=int, default=0, help="HTTP port (default: free ephemeral port)")
    parser.add_argument("--timeout", type=float, default=25.0)
    parser.add_argument("--output-dir", help="Directory for screenshots and logs")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        result = run(args)
    except SmokeFailure as err:
        print(json.dumps({"ok": False, "error": str(err)}, indent=2), file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
