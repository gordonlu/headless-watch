#!/usr/bin/env python3
"""Isolated native Windows acceptance test (Python 3.10+, standard library only).

Build the backend and frontend first, then run ``python tools/windows_smoke.py``.
Use --api-only before the frontend build is available. All run files, browser
profiles, SQLite data and screenshots are placed in a fresh temporary folder.
Only the private browser launched by this script can be passed to terminate.
Existing user browsers are never selected or terminated.
"""

from __future__ import annotations

import argparse
import ctypes
from ctypes import wintypes
import json
import math
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from typing import Callable
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import ProxyHandler, Request, build_opener


ROOT = Path(__file__).resolve().parents[1]
OPEN = build_opener(ProxyHandler({}))


class SmokeFailure(RuntimeError):
    pass


def require(condition: object, message: str) -> None:
    if not condition:
        raise SmokeFailure(message)


def wait_for(probe: Callable, description: str, timeout: float):
    deadline = time.monotonic() + timeout
    last_error = None
    while time.monotonic() < deadline:
        try:
            result = probe()
            if result:
                return result
        except (OSError, URLError, ValueError, SmokeFailure) as error:
            last_error = error
        time.sleep(0.25)
    raise SmokeFailure(f"Timed out waiting for {description}; last error: {last_error}")


def http(base: str, method: str, path: str, token: str | None = None, body=None):
    headers = {"X-Session-Token": token} if token else {}
    data = None
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"
    request = Request(base + path, data=data, headers=headers, method=method)
    try:
        with OPEN.open(request, timeout=12) as response:
            return response.status, dict(response.headers), response.read()
    except HTTPError as error:
        return error.code, dict(error.headers), error.read()


def get_json(base: str, path: str, token: str | None = None):
    status, _, payload = http(base, "GET", path, token)
    require(status == 200, f"GET {path}: HTTP {status}, {payload[:200]!r}")
    return json.loads(payload)


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def resolve_browser(explicit: str | None) -> Path:
    if explicit:
        path = Path(explicit).resolve()
        require(path.is_file(), f"Browser not found: {path}")
        return path
    candidates = []
    for root in ("PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"):
        if os.environ.get(root):
            for suffix in ("Google/Chrome/Application/chrome.exe", "Microsoft/Edge/Application/msedge.exe"):
                candidates.append(Path(os.environ[root], suffix))
    for name in ("chrome.exe", "msedge.exe", "chromium.exe"):
        if shutil.which(name):
            candidates.append(Path(shutil.which(name)))
    for path in candidates:
        if path.is_file():
            return path.resolve()
    raise SmokeFailure("Chrome or Edge not found; pass --browser with an executable path.")


def resolve_binary(explicit: str | None) -> Path:
    candidates = [Path(explicit)] if explicit else [
        ROOT / "backend/target/debug/headless-watch.exe",
        ROOT / "backend/target/release/headless-watch.exe",
    ]
    available = [path for path in candidates if path.is_file()]
    if available:
        return max(available, key=lambda path: path.stat().st_mtime).resolve()
    raise SmokeFailure("Build the native backend first: cargo build --manifest-path backend/Cargo.toml")


class OwnedJob:
    """Windows Job Object containing only children this script starts.

    Closing the job kills remaining child processes, including Chromium
    renderers. Refuse to run when assignment fails instead of falling back to
    broad image-name or process-enumeration based cleanup.
    """

    def __init__(self):
        class IoCounters(ctypes.Structure):
            _fields_ = [(name, ctypes.c_uint64) for name in (
                "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
                "ReadTransferCount", "WriteTransferCount", "OtherTransferCount",
            )]

        class BasicLimits(ctypes.Structure):
            _fields_ = [
                ("PerProcessUserTimeLimit", ctypes.c_int64),
                ("PerJobUserTimeLimit", ctypes.c_int64),
                ("LimitFlags", wintypes.DWORD),
                ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t),
                ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t),
                ("PriorityClass", wintypes.DWORD),
                ("SchedulingClass", wintypes.DWORD),
            ]

        class ExtendedLimits(ctypes.Structure):
            _fields_ = [
                ("BasicLimitInformation", BasicLimits), ("IoInfo", IoCounters),
                ("ProcessMemoryLimit", ctypes.c_size_t), ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t), ("PeakJobMemoryUsed", ctypes.c_size_t),
            ]

        self.kernel = ctypes.WinDLL("kernel32", use_last_error=True)
        self.kernel.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
        self.kernel.CreateJobObjectW.restype = wintypes.HANDLE
        self.kernel.SetInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
        self.kernel.SetInformationJobObject.restype = wintypes.BOOL
        self.kernel.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
        self.kernel.AssignProcessToJobObject.restype = wintypes.BOOL
        self.kernel.CloseHandle.argtypes = [wintypes.HANDLE]
        self.kernel.CloseHandle.restype = wintypes.BOOL
        self.kernel.QueryInformationJobObject.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
        self.kernel.QueryInformationJobObject.restype = wintypes.BOOL
        self.handle = self.kernel.CreateJobObjectW(None, None)
        if not self.handle:
            raise ctypes.WinError(ctypes.get_last_error())
        info = ExtendedLimits()
        info.BasicLimitInformation.LimitFlags = 0x2000  # JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if not self.kernel.SetInformationJobObject(self.handle, 9, ctypes.byref(info), ctypes.sizeof(info)):
            error = ctypes.get_last_error()
            self.close()
            raise ctypes.WinError(error)

    def assign(self, process: subprocess.Popen) -> None:
        if not self.kernel.AssignProcessToJobObject(self.handle, wintypes.HANDLE(int(process._handle))):
            error = ctypes.get_last_error()
            # This process handle identifies our exact child, never a reused PID.
            process.terminate()
            process.wait(timeout=10)
            raise SmokeFailure(f"Cannot isolate the test process in a Windows Job Object: {ctypes.WinError(error)}")

    def close(self) -> None:
        if self.handle:
            self.kernel.CloseHandle(self.handle)
            self.handle = None

    def active_count(self) -> int:
        class BasicAccounting(ctypes.Structure):
            _fields_ = [
                ("TotalUserTime", ctypes.c_int64), ("TotalKernelTime", ctypes.c_int64),
                ("ThisPeriodTotalUserTime", ctypes.c_int64), ("ThisPeriodTotalKernelTime", ctypes.c_int64),
                ("TotalPageFaultCount", wintypes.DWORD), ("TotalProcesses", wintypes.DWORD),
                ("ActiveProcesses", wintypes.DWORD), ("TotalTerminatedProcesses", wintypes.DWORD),
            ]
        accounting = BasicAccounting()
        if not self.kernel.QueryInformationJobObject(self.handle, 1, ctypes.byref(accounting), ctypes.sizeof(accounting), None):
            raise ctypes.WinError(ctypes.get_last_error())
        return int(accounting.ActiveProcesses)

    def process_ids(self) -> list[int]:
        # JOBOBJECT_BASIC_PROCESS_ID_LIST ends with a variable ULONG_PTR array.
        buffer = ctypes.create_string_buffer(8 + 4096 * ctypes.sizeof(ctypes.c_size_t))
        if not self.kernel.QueryInformationJobObject(self.handle, 3, buffer, len(buffer), None):
            raise ctypes.WinError(ctypes.get_last_error())
        count = ctypes.c_uint32.from_buffer(buffer, 4).value
        require(count <= 4096, "Unexpectedly large private test Job Object.")
        return [ctypes.c_size_t.from_buffer(buffer, 8 + i * ctypes.sizeof(ctypes.c_size_t)).value for i in range(count)]


def spawn(command: list[str], log_path: Path, job: OwnedJob) -> subprocess.Popen:
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            command, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=log, stderr=subprocess.STDOUT,
            creationflags=subprocess.CREATE_NO_WINDOW,
        )
    job.assign(process)
    return process


def stop(process: subprocess.Popen | None) -> None:
    if process is not None and process.poll() is None:
        process.terminate()  # Popen retains the exact process handle on Windows.
        process.wait(timeout=10)


def describe_owned_processes(pids: list[int]):
    """Read diagnostics for exact Job-member PIDs only; never signal by name."""
    condition = " OR ".join(f"ProcessId={int(pid)}" for pid in pids)
    if not condition:
        return []
    script = f"Get-CimInstance Win32_Process -Filter '{condition}' | Select-Object ProcessId,ParentProcessId,Name,CommandLine | ConvertTo-Json -Depth 3 -Compress"
    try:
        result = subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
                                capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=10,
                                creationflags=subprocess.CREATE_NO_WINDOW)
        return json.loads(result.stdout) if result.returncode == 0 and result.stdout.strip() else {"unavailable": result.stderr[:300]}
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        return {"unavailable": str(error)}


def run(args) -> tuple[Path, dict]:
    require(sys.platform == "win32", "This acceptance script requires native Windows.")
    binary = resolve_binary(args.binary)
    browser_binary = resolve_browser(args.browser)
    output_parent = Path(args.output_dir).resolve() if args.output_dir else Path(tempfile.gettempdir()).resolve()
    require(not output_parent.is_relative_to(ROOT), "Choose an output directory outside the repository.")
    output_parent.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="headless-watch-windows-", dir=output_parent))
    args.run_output = output
    data_dir = output / "data"
    profile_dir = output / "browser-profile"
    if args.api_only:
        web_dir = output / "web"
        web_dir.mkdir()
        (web_dir / "index.html").write_text('<!doctype html><meta name="hw-token" content="__TOKEN__"><title>Windows smoke</title>', encoding="utf-8")
    else:
        web_dir = Path(args.web_dir).resolve() if args.web_dir else ROOT / "frontend/dist"
        require((web_dir / "index.html").is_file(), "Build the frontend first, or use --api-only.")
    report = {
        "ok": False, "platform": sys.platform, "backend": str(binary),
        "browser": str(browser_binary), "output_dir": str(output), "api_only": args.api_only,
    }
    args.run_report = report
    print(f"Run files: {output}", flush=True)
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    job = OwnedJob()
    server = browser = None

    def start_server(log_name: str):
        process = spawn([
            str(binary), "--port", str(port), "--web-dir", str(web_dir), "--data-dir", str(data_dir),
            "--no-notify", "--interval", "1", "--cdp-interval", "2", "--pss-interval", "5",
        ], output / log_name, job)
        wait_for(lambda: get_json(base, "/api/v1/health") if process.poll() is None else None,
                 "private backend health", args.timeout)
        status, _, html = http(base, "GET", "/")
        match = re.search(r'name="hw-token"\s+content="([^"]+)"', html.decode("utf-8"))
        require(status == 200 and match is not None and match[1] != "__TOKEN__", "Dashboard token was not injected.")
        return process, match[1]

    try:
        server, token = start_server("server.log")
        require(http(base, "GET", "/api/v1/overview")[0] == 403, "Overview accepted a request without a session token.")
        server_job_pids = set(job.process_ids())
        report["private_backend_job_pids"] = sorted(server_job_pids)
        print("[ok] Private server, dashboard token and API token gate", flush=True)

        target_page = output / "target.html"
        target_page.write_text("""<!doctype html><html><head><title>Headless Watch Windows acceptance</title></head>
<body style="margin:0;background:#12243a;color:#ecf5ff;font:28px Segoe UI;padding:64px">
<h1>Windows acceptance</h1><p>Private Chrome / Edge target</p><p>CPU, RSS, CDP and verified cleanup</p>
<script>setInterval(()=>{const end=performance.now()+35;while(performance.now()<end){}},100);</script>
</body></html>""", encoding="utf-8")
        cdp_port = free_port()
        browser = spawn([
            str(browser_binary), "--headless=new", "--disable-gpu", "--no-first-run", "--no-default-browser-check",
            "--disable-background-networking", "--disable-extensions", f"--remote-debugging-port={cdp_port}",
            f"--user-data-dir={profile_dir}", "--window-size=1280,800", target_page.as_uri(),
        ], output / "browser.log", job)
        report["browser_launcher_pid"] = browser.pid
        cdp_version = wait_for(lambda: get_json(f"http://127.0.0.1:{cdp_port}", "/json/version"),
                               "private browser CDP", args.timeout)
        report["browser_version"] = cdp_version.get("Browser")

        def own_session():
            overview = get_json(base, "/api/v1/overview", token)
            owned_pids = set(job.process_ids()) - server_job_pids
            # Edge may hand off from its initial launcher to a new root PID.
            # The candidate must still belong to our isolated Job and expose
            # the exact CDP endpoint we allocated to the private browser.
            return next((s for s in overview.get("sessions", [])
                         if s.get("pid") in owned_pids and s.get("cdp", {}).get("port") == cdp_port), None)

        session = wait_for(own_session, "Windows native process discovery", args.timeout)
        target_pid = session["pid"]
        session_id = str(session["id"])
        encoded_id = quote(session_id, safe="")
        require(session["identity"]["pid"] == target_pid, "Discovered identity does not match our exact child PID.")
        ticks = session["identity"]["start_ticks"]
        require(isinstance(ticks, str) or (0 <= ticks <= 2**53 - 1 and int(float(ticks)) == ticks),
                "Numeric start_ticks loses precision after JavaScript JSON.parse; UI termination cannot preserve identity.")
        report.update({"session_id": session_id, "pid": target_pid, "identity": session["identity"]})
        print(f"[ok] Native discovery of private browser PID {target_pid}", flush=True)

        observations = []

        def sampled_session():
            sample = own_session()
            if not sample:
                return None
            cpu = sample.get("cpu", {})
            history = sample.get("history", [])
            if cpu.get("cores") is not None and cpu.get("quality") != "first_sample" and len(history) >= 2:
                observations.append({"cpu": cpu, "memory": sample.get("memory"), "history_count": len(history)})
                return sample
            return None

        session = wait_for(sampled_session, "two real CPU delta samples", args.timeout)
        cpu = session["cpu"]
        require(math.isfinite(cpu["cores"]) and cpu["cores"] > 0, f"CPU probe did not produce a positive finite sample: {cpu}")
        require(session["memory"]["rss_sum"] > 0, "Native working-set RSS is empty.")

        def unsupported_pss():
            sample = own_session()
            memory = sample.get("memory", {}) if sample else {}
            return memory if memory.get("pss_quality") == "unsupported" else None

        memory = wait_for(unsupported_pss, "explicit unsupported Windows PSS", args.timeout)
        require(memory.get("pss_sum") is None, "Unsupported PSS must stay null instead of reporting fabricated data.")
        metrics = wait_for(lambda: (p if len(p := get_json(base, f"/api/v1/sessions/{encoded_id}/metrics?window=30m", token)["points"]) >= 2 else None),
                           "persisted CPU samples", args.timeout)
        report["samples"] = observations
        report["metrics_before_restart"] = len(metrics)
        report["pss"] = memory
        print("[ok] CPU deltas, working-set RSS, two persisted metrics and explicit unsupported PSS", flush=True)

        def cdp_session():
            sample = own_session()
            if not sample:
                return None
            tabs = sample.get("cdp", {}).get("tabs") or []
            if any("Windows acceptance" in tab.get("title", "") for tab in tabs):
                return sample
            return None

        session = wait_for(cdp_session, "backend CDP tab discovery", args.timeout)
        report["cdp"] = session.get("cdp", {})
        tab = next(t for t in session["cdp"]["tabs"] if "Windows acceptance" in t.get("title", ""))
        status, headers, image = http(base, "POST", f"/api/v1/sessions/{encoded_id}/screenshot", token, {"tab_id": tab["id"]})
        require(status == 200 and image.startswith(b"\xff\xd8\xff"), f"CDP screenshot did not return JPEG: HTTP {status}")
        (output / "cdp-screenshot.jpg").write_bytes(image)
        report["screenshot"] = {"bytes": len(image), "content_type": headers.get("content-type", headers.get("Content-Type"))}
        print("[ok] CDP page metadata and real JPEG capture", flush=True)

        # A kill request is allowed only after matching our child's PID and CDP
        # target. The stale request changes one identity field, never its PID.
        require(target_pid in job.process_ids() and session["identity"]["pid"] == target_pid,
                "Refusing termination because the private child identity no longer matches.")
        wrong_identity = dict(session["identity"])
        wrong_identity["start_ticks"] = str(int(ticks) + 1) if isinstance(ticks, str) else ticks + 1
        status, _, rejected = http(base, "POST", f"/api/v1/sessions/{encoded_id}/terminate", token,
                                   {"identity": wrong_identity, "mode": "kill", "confirm": True})
        require(status == 409, f"Stale identity was not rejected: HTTP {status}, {rejected[:200]!r}")
        require(target_pid in job.process_ids(), "Stale identity terminated the private browser.")
        report["job_pids_before_browser_kill"] = job.process_ids()
        report["browser_tree_before_kill"] = session.get("processes", [])
        status, _, terminated = http(base, "POST", f"/api/v1/sessions/{encoded_id}/terminate", token,
                                     {"identity": session["identity"], "mode": "kill", "confirm": True})
        outcome = json.loads(terminated)
        require(status == 200 and outcome.get("ok") is True, f"Verified private browser kill failed: HTTP {status}, {outcome}")
        browser.wait(timeout=args.timeout)
        require(not outcome.get("survivors"), f"Termination left private browser survivors: {outcome}")
        report["termination"] = outcome
        try:
            wait_for(lambda: set(job.process_ids()).issubset(server_job_pids),
                     "no private browser descendants after verified kill", args.timeout)
        finally:
            report["job_pids_after_browser_kill"] = job.process_ids()
            if not set(report["job_pids_after_browser_kill"]).issubset(server_job_pids):
                report["remaining_private_processes"] = describe_owned_processes(report["job_pids_after_browser_kill"])
        print("[ok] Stale identity rejected; verified kill terminated only the private browser", flush=True)

        def ended_session():
            ended = get_json(base, "/api/v1/sessions?status=ended&limit=1000", token)
            return next((s for s in ended.get("sessions", []) if s.get("id") == session_id), None)

        ended = wait_for(ended_session, "ended session persistence", args.timeout)
        first_seen = ended["first_seen_at"]
        stop(server)
        server, token = start_server("server-restart.log")
        ended_after = wait_for(ended_session, "ended session after backend restart", args.timeout)
        require(ended_after["first_seen_at"] == first_seen, "History first_seen_at changed on restart.")
        metrics_after = get_json(base, f"/api/v1/sessions/{encoded_id}/metrics?window=30m", token)["points"]
        require(len(metrics_after) >= len(metrics), "Metric samples did not survive a backend restart.")
        actions = get_json(base, "/api/v1/actions?limit=1000", token).get("actions", [])
        own_actions = [a for a in actions if a.get("session_id") == session_id]
        require(len(own_actions) >= 2, "Rejected and successful actions did not survive restart.")
        report.update({"ok": True, "history_persisted": True, "metrics_after_restart": len(metrics_after), "action_count": len(own_actions)})
        print("[ok] Ended history, metrics and action audit survive backend restart", flush=True)
        return output, report
    finally:
        # The job can contain only processes launched through spawn() above.
        job.close()
        for process in (browser, server):
            if process is not None:
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    stop(process)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", help="Native Windows backend executable")
    parser.add_argument("--browser", help="Chrome/Edge executable; common install paths are auto-detected")
    parser.add_argument("--web-dir", help="Built frontend directory (default: frontend/dist)")
    parser.add_argument("--api-only", action="store_true", help="Use a temporary token HTML instead of the dashboard build")
    parser.add_argument("--output-dir", help="Parent directory for a fresh evidence folder; must be outside the repository")
    parser.add_argument("--timeout", type=float, default=45, help="Per-stage timeout in seconds")
    args = parser.parse_args()
    try:
        output, report = run(args)
    except (SmokeFailure, OSError, ValueError, subprocess.SubprocessError) as error:
        output = getattr(args, "run_output", None)
        report = getattr(args, "run_report", {"ok": False, "output_dir": str(output) if output else None})
        report["error"] = str(error)
        if output:
            (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(json.dumps(report, ensure_ascii=False, indent=2), file=sys.stderr)
        return 1
    (output / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
