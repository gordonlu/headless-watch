#!/usr/bin/env python3
"""Repeatable idle-backend CPU/RSS benchmark.

Starts a private Headless Watch server, warms up, then measures its own CPU
time and RSS for a fixed window. Existing browser sessions are monitored as
usual. Use --max-cpu to turn the measurement into a regression gate.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
from urllib.request import ProxyHandler, Request, build_opener

ROOT = Path(__file__).resolve().parents[1]
OPEN = build_opener(ProxyHandler({}))
for key in ("http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"):
    os.environ.pop(key, None)
os.environ["NO_PROXY"] = "127.0.0.1,localhost"


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def cpu_ticks(pid: int) -> int:
    raw = Path(f"/proc/{pid}/stat").read_text(encoding="utf-8", errors="replace")
    fields = raw[raw.rfind(")") + 2 :].split()
    return int(fields[11]) + int(fields[12])


def rss_kib(pid: int) -> int:
    for line in Path(f"/proc/{pid}/status").read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("VmRSS:"):
            return int(line.split()[1])
    return 0


def wait_health(base: str, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            request = Request(base + "/api/v1/health")
            with OPEN.open(request, timeout=1) as response:
                if response.status == 200:
                    return
        except Exception:
            pass
        time.sleep(0.15)
    raise RuntimeError("server did not become healthy")


def resolve_binary(explicit: str | None) -> Path:
    if explicit:
        return Path(explicit).resolve()
    for candidate in (
        ROOT / "backend/target/release/headless-watch",
        ROOT / "backend/target/debug/headless-watch",
    ):
        if candidate.is_file():
            return candidate
    raise RuntimeError("build the backend first: cargo build --release --manifest-path backend/Cargo.toml")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary")
    parser.add_argument("--web-dir", default=str(ROOT / "frontend/dist"))
    parser.add_argument("--seconds", type=float, default=60.0)
    parser.add_argument("--warmup", type=float, default=5.0)
    parser.add_argument("--max-cpu", type=float, help="Fail if single-core CPU percent exceeds this")
    parser.add_argument("--output-dir")
    args = parser.parse_args()

    binary = resolve_binary(args.binary)
    web_dir = Path(args.web_dir).resolve()
    if not (web_dir / "index.html").is_file():
        print(f"frontend build not found in {web_dir}", file=sys.stderr)
        return 1
    output_dir = Path(args.output_dir) if args.output_dir else Path(tempfile.mkdtemp(prefix="hw-benchmark-"))
    output_dir.mkdir(parents=True, exist_ok=True)
    data_dir = Path(tempfile.mkdtemp(prefix="hw-benchmark-data-"))
    port = free_port()
    base = f"http://127.0.0.1:{port}"
    log = (output_dir / "server.log").open("wb")
    server = subprocess.Popen(
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
        ],
        cwd=ROOT,
        stdin=subprocess.DEVNULL,
        stdout=log,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )
    try:
        wait_health(base, 20)
        time.sleep(args.warmup)
        before = cpu_ticks(server.pid)
        started = time.monotonic()
        time.sleep(args.seconds)
        after = cpu_ticks(server.pid)
        elapsed = max(0.001, time.monotonic() - started)
        clk = os.sysconf(os.sysconf_names["SC_CLK_TCK"])
        cpu = (after - before) / clk / elapsed * 100.0
        result = {
            "cpu_single_core_pct": round(cpu, 3),
            "rss_mib": round(rss_kib(server.pid) / 1024.0, 2),
            "seconds": round(elapsed, 2),
            "target_cpu_pct": 1.0,
            "passed": cpu < 1.0,
            "binary": str(binary),
            "log": str(output_dir / "server.log"),
        }
        print(json.dumps(result, indent=2))
        if args.max_cpu is not None and cpu > args.max_cpu:
            print(
                f"CPU regression: {cpu:.3f}% > {args.max_cpu:.3f}%",
                file=sys.stderr,
            )
            return 1
        return 0
    finally:
        if server.poll() is None:
            try:
                os.killpg(server.pid, signal.SIGTERM)
            except (ProcessLookupError, PermissionError, OSError):
                pass
            try:
                server.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(server.pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError, OSError):
                    pass
        log.close()
        shutil.rmtree(data_dir, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
