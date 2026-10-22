#!/usr/bin/env python3
"""Linux-only acceptance checks for the Rust backend.

Covers scenarios that need real processes but no dashboard browser:

1. Force kill: a fake headless browser root that ignores SIGTERM must survive
   Stop, be reported as still_running, and be killed only by a confirmed kill.
2. Crash convergence: a real headless Chrome is SIGKILLed; the session must
   leave the live list and appear in ended history and Recently exited.
3. Port conflict: a second server on the same port must exit with a readable
   error, and stopping the first server must leave no orphan process.
4. Resource sample: reports backend CPU and RSS while sessions are monitored.
"""

from __future__ import annotations

import argparse
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
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

import websocket

ROOT = Path(__file__).resolve().parents[1]
OPEN = build_opener(ProxyHandler({}))
for key in ("http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
    os.environ.pop(key, None)
os.environ["NO_PROXY"] = "127.0.0.1,localhost"


class AcceptanceFailure(RuntimeError):
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
    headers: dict[str, str] = {}
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
            status, _, _, _ = http_json(base, "GET", "/api/v1/health")
            if status == 200:
                return
        except Exception as err:
            last_error = err
        time.sleep(0.15)
    raise AcceptanceFailure(f"server did not become healthy: {last_error}")


def fetch_token(base: str) -> str:
    status, _, page = http_request(base, "GET", "/")
    if status != 200:
        raise AcceptanceFailure(f"dashboard returned HTTP {status}")
    match = re.search(r'name="hw-token" content="([^"]+)"', page.decode("utf-8"))
    if not match or match.group(1) == "__TOKEN__":
        raise AcceptanceFailure("dashboard token was not injected")
    return match.group(1)


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


def start_server(binary: Path, port: int, web_dir: Path, data_dir: Path, log_path: Path) -> subprocess.Popen:
    log = log_path.open("wb")
    return subprocess.Popen(
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
            "15",
            "--leftover-after",
            "10",
        ],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )


def resolve_binary(explicit: str | None) -> Path:
    if explicit:
        path = Path(explicit).resolve()
        if not path.is_file():
            raise AcceptanceFailure(f"binary not found: {path}")
        return path
    for candidate in (
        ROOT / "backend/target/release/headless-watch",
        ROOT / "backend/target/debug/headless-watch",
    ):
        if candidate.is_file():
            return candidate
    raise AcceptanceFailure("build the backend first: cargo build --release --manifest-path backend/Cargo.toml")


def compile_fake_browser(directory: Path) -> Path | None:
    compiler = shutil.which("cc") or shutil.which("gcc")
    if not compiler:
        return None
    source = directory / "fake_chrome.c"
    binary = directory / "chrome"
    source.write_text(
        "#include <signal.h>\n"
        "#include <stdlib.h>\n"
        "#include <string.h>\n"
        "#include <unistd.h>\n"
        "int main(int argc, char **argv){\n"
        "  int children = 0;\n"
        "  for (int i = 1; i < argc; i++) {\n"
        "    if (!strncmp(argv[i], \"--children=\", 11)) children = atoi(argv[i] + 11);\n"
        "  }\n"
        "  signal(SIGTERM, SIG_IGN);\n"
        "  for (int i = 0; i < children; i++) { if (fork() == 0) { for(;;) pause(); } }\n"
        "  for(;;){pause();}\n"
        "}\n",
        encoding="utf-8",
    )
    result = subprocess.run([compiler, "-O2", "-o", str(binary), str(source)], capture_output=True, text=True)
    if result.returncode != 0:
        return None
    return binary


def wait_for_session(base: str, token: str, pid: int, timeout: float) -> dict:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        status, _, payload, _ = http_json(base, "GET", "/api/v1/overview", token)
        if status == 200:
            session = next((item for item in payload.get("sessions", []) if item.get("pid") == pid), None)
            if session:
                return session
        time.sleep(0.25)
    raise AcceptanceFailure(f"session for PID {pid} was not discovered")


def wait_for_session_gone(base: str, token: str, pid: int, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        status, _, payload, _ = http_json(base, "GET", "/api/v1/overview", token)
        if status == 200 and not any(item.get("pid") == pid for item in payload.get("sessions", [])):
            return
        time.sleep(0.25)
    raise AcceptanceFailure(f"session for PID {pid} did not leave the live list")


def process_cpu_ticks(pid: int) -> tuple[int, int]:
    raw = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
    close = raw.rfind(")")
    fields = raw[close + 2 :].split()
    return int(fields[11]), int(fields[12])


def process_rss_kib(pid: int) -> int:
    for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    return 0


def run_cross_site_test(
    watch_base: str,
    watch_port: int,
    chrome_binary: str | None,
    timeout: float,
) -> dict[str, object]:
    if not chrome_binary:
        return {"skipped": "Chrome/Chromium not installed"}
    attacker_port_holder: dict[str, int] = {}

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self) -> None:  # noqa: N802
            watch = watch_base
            page = f"""<!doctype html><html><body><script>
window.results = {{}};
fetch('{watch}/api/v1/overview')
  .then(r => {{ window.results.overview = r.status; }})
  .catch(() => {{ window.results.overview = 'blocked'; }});
fetch('{watch}/api/v1/sessions/victim/terminate', {{
  method: 'POST',
  headers: {{ 'Content-Type': 'application/json' }},
  body: JSON.stringify({{ identity: {{ pid: 1, start_ticks: 1 }}, mode: 'term', confirm: true }})
}})
  .then(r => {{ window.results.terminate = r.status; }})
  .catch(() => {{ window.results.terminate = 'blocked'; }});
</script></body></html>"""
            data = page.encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, *_: object) -> None:
            return

    attacker = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    attacker_port_holder["port"] = attacker.server_address[1]
    thread = threading.Thread(target=attacker.serve_forever, daemon=True)
    thread.start()
    profile = tempfile.TemporaryDirectory(prefix="hw-cross-site-profile-")
    chrome: subprocess.Popen | None = None
    ws = None
    try:
        chrome = subprocess.Popen(
            [
                chrome_binary,
                "--headless=new",
                "--disable-gpu",
                "--disable-dev-shm-usage",
                "--no-first-run",
                "--no-default-browser-check",
                "--remote-debugging-port=0",
                f"--user-data-dir={profile.name}",
                "about:blank",
            ],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
        port_file = Path(profile.name, "DevToolsActivePort")
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline and not port_file.exists():
            if chrome.poll() is not None:
                raise AcceptanceFailure("cross-site Chrome exited before exposing CDP")
            time.sleep(0.1)
        cdp_port = int(port_file.read_text(encoding="ascii").splitlines()[0])
        targets = json.loads(
            OPEN.open(f"http://127.0.0.1:{cdp_port}/json/list", timeout=2).read()
        )
        page_target = next((item for item in targets if item.get("type") == "page"), None)
        if not page_target:
            raise AcceptanceFailure("cross-site Chrome exposed no page target")
        ws = websocket.create_connection(
            str(page_target["webSocketDebuggerUrl"]).replace("localhost", "127.0.0.1"),
            timeout=5,
            suppress_origin=True,
            http_proxy_host=None,
        )
        ws.settimeout(0.25)
        request_id = iter(range(1, 10000))

        def command(method: str, params: dict | None = None) -> dict:
            cid = next(request_id)
            ws.send(json.dumps({"id": cid, "method": method, "params": params or {}}))
            deadline = time.monotonic() + 8
            while time.monotonic() < deadline:
                try:
                    message = json.loads(ws.recv())
                except websocket.WebSocketTimeoutException:
                    continue
                if message.get("id") == cid:
                    if "error" in message:
                        raise AcceptanceFailure(f"CDP {method}: {message['error']}")
                    return message.get("result", {})
            raise AcceptanceFailure(f"CDP {method} timed out")

        command("Runtime.enable")
        command("Page.enable")
        command("Page.navigate", {"url": f"http://127.0.0.1:{attacker_port_holder['port']}/"})
        results: dict[str, object] = {}
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            evaluation = command(
                "Runtime.evaluate",
                {
                    "expression": "JSON.stringify(window.results || {})",
                    "returnByValue": True,
                },
            )
            results = json.loads(evaluation["result"].get("value") or "{}")
            if "overview" in results and "terminate" in results:
                break
            time.sleep(0.2)
        if results.get("overview") != "blocked" or results.get("terminate") != "blocked":
            raise AcceptanceFailure(f"cross-site page was not fully blocked: {results}")
        return {"attacker_origin": f"http://127.0.0.1:{attacker_port_holder['port']}", "results": results}
    finally:
        if ws is not None:
            try:
                ws.close()
            except Exception:
                pass
        if chrome is not None:
            terminate_group(chrome)
        profile.cleanup()
        attacker.shutdown()
        attacker.server_close()


def run(args: argparse.Namespace) -> dict[str, object]:
    binary = resolve_binary(args.binary)
    web_dir = (Path(args.web_dir) if args.web_dir else ROOT / "frontend/dist").resolve()
    if not (web_dir / "index.html").is_file():
        raise AcceptanceFailure(f"frontend build not found in {web_dir}; run pnpm --dir frontend build")
    output_dir = Path(args.output_dir) if args.output_dir else Path(tempfile.mkdtemp(prefix="hw-linux-acceptance-"))
    output_dir.mkdir(parents=True, exist_ok=True)
    data_dir = Path(tempfile.mkdtemp(prefix="hw-linux-acceptance-data-"))
    profile = tempfile.TemporaryDirectory(prefix="hw-linux-acceptance-profile-")
    port = args.port or free_port()
    base = f"http://127.0.0.1:{port}"
    server: subprocess.Popen | None = None
    fake: subprocess.Popen | None = None
    stress: subprocess.Popen | None = None
    chrome: subprocess.Popen | None = None
    launcher: subprocess.Popen | None = None
    lifecycle_profile = tempfile.TemporaryDirectory(prefix="hw-lifecycle-profile-")
    result: dict[str, object] = {"ok": False}
    try:
        server = start_server(binary, port, web_dir, data_dir, output_dir / "server.log")
        wait_health(base, args.timeout)
        token = fetch_token(base)
        result["server"] = {"pid": server.pid, "port": port}

        # 1. Force kill against a process that ignores SIGTERM.
        fake_binary = compile_fake_browser(output_dir)
        if fake_binary is None:
            result["force_kill"] = {"skipped": "cc/gcc not available"}
        else:
            fake = subprocess.Popen(
                [str(fake_binary), "--headless=new", "--remote-debugging-pipe"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            session = wait_for_session(base, token, fake.pid, args.timeout)
            stale_identity = dict(session["identity"])
            stale_identity["start_ticks"] = int(stale_identity["start_ticks"]) + 1
            stale_status, _, stale_body, _ = http_json(
                base,
                "POST",
                f"/api/v1/sessions/{session['id']}/terminate",
                token,
                {"identity": stale_identity, "mode": "term", "confirm": True},
            )
            if stale_status != 409 or (stale_body or {}).get("ok") is not False:
                raise AcceptanceFailure(
                    f"stale identity was not rejected: {stale_status} {stale_body}"
                )
            if fake.poll() is not None:
                raise AcceptanceFailure("stale identity request terminated the process")
            term_status, _, term_body, _ = http_json(
                base,
                "POST",
                f"/api/v1/sessions/{session['id']}/terminate",
                token,
                {"identity": session["identity"], "mode": "term", "confirm": True},
            )
            if fake.poll() is not None:
                raise AcceptanceFailure("fake browser exited after SIGTERM despite ignoring it")
            if term_status not in {200, 409} or (term_body or {}).get("result") != "still_running":
                raise AcceptanceFailure(f"SIGTERM result was not still_running: {term_status} {term_body}")
            kill_status, _, kill_body, _ = http_json(
                base,
                "POST",
                f"/api/v1/sessions/{session['id']}/terminate",
                token,
                {"identity": session["identity"], "mode": "kill", "confirm": True},
            )
            fake.wait(timeout=6)
            if kill_status != 200 or not (kill_body or {}).get("ok"):
                raise AcceptanceFailure(f"force kill failed: {kill_status} {kill_body}")
            status, _, actions, _ = http_json(base, "GET", "/api/v1/actions?limit=50", token)
            rows = (actions or {}).get("actions", [])
            result["force_kill"] = {
                "stale_identity_rejected": True,
                "term": next((row.get("result") for row in rows if row.get("action") == "term"), None),
                "kill": next((row.get("result") for row in rows if row.get("action") == "kill"), None),
            }

        # 2. Crash convergence with a real headless Chrome.
        chrome_binary = shutil.which("google-chrome") or shutil.which("chromium")
        if not chrome_binary:
            result["crash_convergence"] = {"skipped": "Chrome/Chromium not installed"}
        else:
            chrome = subprocess.Popen(
                [
                    chrome_binary,
                    "--headless=new",
                    "--disable-gpu",
                    "--disable-dev-shm-usage",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--remote-debugging-port=0",
                    f"--user-data-dir={profile.name}",
                    "about:blank",
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            session = wait_for_session(base, token, chrome.pid, args.timeout)
            session_id = session["id"]
            os.killpg(chrome.pid, signal.SIGKILL)
            chrome.wait(timeout=5)
            wait_for_session_gone(base, token, chrome.pid, args.timeout)
            status, _, ended_payload, _ = http_json(base, "GET", "/api/v1/sessions?status=ended&limit=50", token)
            ended = next(
                (item for item in (ended_payload or {}).get("sessions", []) if item.get("id") == session_id),
                None,
            )
            status, _, overview, _ = http_json(base, "GET", "/api/v1/overview", token)
            recent = next(
                (item for item in (overview or {}).get("recent_ended", []) if item.get("id") == session_id),
                None,
            )
            if ended is None or recent is None:
                raise AcceptanceFailure("crashed session was not persisted in history")
            result["crash_convergence"] = {
                "ended_at": ended.get("ended_at"),
                "recent_exited": True,
            }

        # 3. Real launcher exit lifecycle (AT-06).
        chrome_binary = shutil.which("google-chrome") or shutil.which("chromium")
        if not chrome_binary:
            result["lifecycle"] = {"skipped": "Chrome/Chromium not installed"}
        else:
            pidfile = output_dir / "lifecycle-chrome.pid"
            mcp_script = output_dir / "mcp.py"
            launcher_script = output_dir / "launcher.py"
            mcp_script.write_text(
                "import os, subprocess, time\n"
                "chrome = os.environ['HW_CHROME']\n"
                "profile = os.environ['HW_PROFILE']\n"
                "pidfile = os.environ['HW_PIDFILE']\n"
                "cmd = [chrome, '--headless=new', '--disable-gpu', '--disable-dev-shm-usage',\n"
                "       '--no-first-run', '--no-default-browser-check', '--remote-debugging-port=0',\n"
                "       '--user-data-dir=' + profile, 'about:blank']\n"
                "p = subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
                "open(pidfile, 'w').write(str(p.pid))\n"
                "while True: time.sleep(1)\n",
                encoding="utf-8",
            )
            launcher_script.write_text(
                "import os, subprocess, time\n"
                "python = os.environ['HW_PYTHON']\n"
                "mcp = os.environ['HW_MCP_SCRIPT']\n"
                "subprocess.Popen(['bash', '-c', f'exec -a node {python} {mcp} playwright-mcp'],\n"
                "                 stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n"
                "while True: time.sleep(1)\n",
                encoding="utf-8",
            )
            env = dict(os.environ)
            env.update(
                HW_CHROME=chrome_binary,
                HW_PROFILE=lifecycle_profile.name,
                HW_PIDFILE=str(pidfile),
                HW_PYTHON=shutil.which("python3"),
                HW_MCP_SCRIPT=str(mcp_script),
            )
            launcher = subprocess.Popen(
                [
                    "bash",
                    "-c",
                    f"exec -a opencode {shutil.which('python3')} {launcher_script}",
                ],
                env=env,
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline and not pidfile.exists():
                if launcher.poll() is not None:
                    raise AcceptanceFailure("opencode launcher exited before starting Chrome")
                time.sleep(0.1)
            if not pidfile.exists():
                raise AcceptanceFailure("opencode launcher did not start Chrome")
            lifecycle_pid = int(pidfile.read_text(encoding="utf-8"))
            lifecycle_session = wait_for_session(base, token, lifecycle_pid, args.timeout)
            source = lifecycle_session.get("source", {})
            if source.get("launcher") != "OpenCode" or source.get("driver") != "Playwright":
                raise AcceptanceFailure(f"launcher/driver ancestry was not separated: {source}")
            os.kill(launcher.pid, signal.SIGKILL)
            launcher.wait(timeout=5)
            deadline = time.monotonic() + args.timeout
            observed = None
            while time.monotonic() < deadline:
                status, _, payload, _ = http_json(base, "GET", "/api/v1/overview", token)
                if status == 200:
                    observed = next(
                        (item for item in payload.get("sessions", []) if item.get("pid") == lifecycle_pid),
                        None,
                    )
                    if observed and observed.get("status", {}).get("launcher_exited"):
                        break
                time.sleep(0.3)
            if not observed or not observed.get("status", {}).get("launcher_exited"):
                raise AcceptanceFailure("launcher exit was not observed")
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                status, _, payload, _ = http_json(base, "GET", "/api/v1/overview", token)
                if status == 200:
                    observed = next(
                        (item for item in payload.get("sessions", []) if item.get("pid") == lifecycle_pid),
                        None,
                    )
                    if observed and observed.get("status", {}).get("suspected_leftover"):
                        break
                time.sleep(0.5)
            if not observed or not observed.get("status", {}).get("suspected_leftover"):
                raise AcceptanceFailure("suspected leftover was not raised after the delay")
            http_json(
                base,
                "POST",
                f"/api/v1/sessions/{observed['id']}/terminate",
                token,
                {"identity": observed["identity"], "mode": "term", "confirm": True},
            )
            result["lifecycle"] = {
                "launcher": observed.get("source", {}).get("launcher"),
                "driver": observed.get("source", {}).get("driver"),
                "launcher_exited": True,
                "parent_alive": not observed.get("parent", {}).get("exited", True),
                "suspected_leftover": True,
            }

        # 4. Real 100-child browser tree (AT-18).
        if fake_binary is None:
            result["stress_100"] = {"skipped": "cc/gcc not available"}
        else:
            stress = subprocess.Popen(
                [
                    str(fake_binary),
                    "--headless=new",
                    "--remote-debugging-pipe",
                    "--children=100",
                ],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
            stress_session = None
            deadline = time.monotonic() + args.timeout
            while time.monotonic() < deadline:
                status, _, payload, _ = http_json(base, "GET", "/api/v1/overview", token)
                if status == 200:
                    candidate = next(
                        (item for item in payload.get("sessions", []) if item.get("pid") == stress.pid),
                        None,
                    )
                    if candidate and candidate.get("process_count", 0) >= 100:
                        stress_session = candidate
                        break
                time.sleep(0.3)
            if stress_session is None:
                raise AcceptanceFailure("the 100-child browser tree was not discovered with 100 processes")
            latencies = []
            for _ in range(10):
                started = time.monotonic()
                http_json(base, "GET", "/api/v1/overview", token)
                latencies.append((time.monotonic() - started) * 1000.0)
            before = process_cpu_ticks(server.pid)
            started = time.monotonic()
            time.sleep(5.0)
            after = process_cpu_ticks(server.pid)
            elapsed = max(0.001, time.monotonic() - started)
            ticks = (after[0] + after[1]) - (before[0] + before[1])
            clk = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
            kill_status, _, kill_body, _ = http_json(
                base,
                "POST",
                f"/api/v1/sessions/{stress_session['id']}/terminate",
                token,
                {"identity": stress_session["identity"], "mode": "kill", "confirm": True},
            )
            stress.wait(timeout=8)
            if kill_status != 200 or not (kill_body or {}).get("ok"):
                raise AcceptanceFailure(f"stress force kill failed: {kill_status} {kill_body}")
            result["stress_100"] = {
                "process_count": stress_session.get("process_count"),
                "overview_max_ms": round(max(latencies), 1),
                "overview_avg_ms": round(sum(latencies) / len(latencies), 1),
                "cpu_single_core_pct": round(ticks / clk / elapsed * 100.0, 3),
                "rss_mib": round(process_rss_kib(server.pid) / 1024.0, 2),
            }

        # 4. Resource sample while the server is monitoring.
        before = process_cpu_ticks(server.pid)
        started = time.monotonic()
        time.sleep(5.0)
        after = process_cpu_ticks(server.pid)
        elapsed = max(0.001, time.monotonic() - started)
        ticks = (after[0] + after[1]) - (before[0] + before[1])
        clk = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
        result["resources"] = {
            "cpu_single_core_pct": round(ticks / clk / elapsed * 100.0, 3),
            "rss_mib": round(process_rss_kib(server.pid) / 1024.0, 2),
            "elapsed_s": round(elapsed, 2),
        }

        # 5. Cross-site page attempt (AT-13).
        result["cross_site"] = run_cross_site_test(
            base,
            port,
            shutil.which("google-chrome") or shutil.which("chromium"),
            args.timeout,
        )

        terminate_group(server)
        server = None

        # 6. Port conflict: the first server is stopped, so start two on one port.
        conflict_port = free_port()
        first = start_server(binary, conflict_port, web_dir, data_dir, output_dir / "conflict-first.log")
        conflict_base = f"http://127.0.0.1:{conflict_port}"
        wait_health(conflict_base, args.timeout)
        second = start_server(binary, conflict_port, web_dir, data_dir, output_dir / "conflict-second.log")
        second.wait(timeout=args.timeout)
        second_log = (output_dir / "conflict-second.log").read_text(encoding="utf-8", errors="replace")
        if second.returncode == 0:
            terminate_group(second)
            raise AcceptanceFailure("a second server unexpectedly bound the same port")
        if "cannot bind" not in second_log and "Address already in use" not in second_log:
            raise AcceptanceFailure(f"port conflict error was not readable: {second_log[-200:]}")
        terminate_group(first)
        first.wait(timeout=5)
        if first.poll() is None:
            raise AcceptanceFailure("the first server did not exit cleanly")
        result["port_conflict"] = {"second_exit": second.returncode, "readable_error": True}

        result["ok"] = True
        return result
    finally:
        for process in (fake, stress, chrome, launcher, server):
            if process is not None:
                terminate_group(process)
        if launcher is not None:
            try:
                os.killpg(launcher.pid, signal.SIGKILL)
            except (ProcessLookupError, PermissionError, OSError):
                pass
        profile.cleanup()
        lifecycle_profile.cleanup()
        shutil.rmtree(data_dir, ignore_errors=True)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", help="Rust binary (default: release, then debug)")
    parser.add_argument("--web-dir", help="Built frontend directory (default: frontend/dist)")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--timeout", type=float, default=20.0)
    parser.add_argument("--output-dir", help="Directory for logs")
    return parser


def main() -> int:
    args = build_parser().parse_args()
    try:
        result = run(args)
    except AcceptanceFailure as err:
        print(json.dumps({"ok": False, "error": str(err)}, indent=2), file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
