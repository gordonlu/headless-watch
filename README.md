# Headless Watch

A local dashboard and history service for headless Chrome/Chromium browsers on Linux and Windows. It observes native process information and existing Chrome DevTools endpoints without modifying, restarting, or attaching input to the tools that launched the browser.

**v1.0** is the Rust/Tokio collector, Axum HTTP + SSE API, SQLite WAL history, and Vue 3 dashboard. The Python v0.2 implementation was removed after migration and is not part of the public source tree.

## Status

v1.0 Linux acceptance is complete. The implementation and prebuilt archive are tracked in [`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md); the PRD remains in `docs/` as the design record.

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — modules, data flow, storage schema, failure boundaries.
- [`docs/SECURITY.md`](docs/SECURITY.md) — threat model, controls, and explicit limits.
- [`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md) — acceptance criteria status.
- [`docs/Headless-Watch-PRD-v1.0.md`](docs/Headless-Watch-PRD-v1.0.md) — original product requirements.

## Platform support

| Capability | Linux | Windows |
|---|---|---|
| Process identity / CPU / memory | `/proc`, CPU ticks, RSS | Native process handles, creation FILETIME, CPU time, working set |
| Proportional memory (PSS) | `smaps_rollup` | Unavailable; shown as unavailable rather than estimated |
| Login startup | systemd user service | Current-user Startup shortcut; no administrator rights |
| Stop browser subtree | Verified SIGTERM / SIGKILL | Verified force stop (`TerminateProcess`); graceful browser stop unavailable |
| Desktop notifications | Optional `notify-send` | Not yet implemented; alerts remain in the dashboard |
| Default data | XDG data path | `%LOCALAPPDATA%\headless-watch\data` |

The original Linux acceptance was performed on Ubuntu (Wayland, 16 online CPUs, Chrome 155). The repository now includes Linux and Windows build/test CI; see [Windows setup and limits](docs/WINDOWS.md) for native installation and validation. macOS is not supported. The historical PRD describes the original Linux scope.

### Linux completion before v1.0

The remaining Linux-only gates are tracked in [`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md):

- real `SIGKILL` Force-kill scenario and a real PID-reuse race;
- a real cross-site page attempting to call the local API;
- a real browser crash convergence run;
- port-conflict and no-orphan-monitor automation;
- a real 100-child browser stress run with backend CPU/RSS and UI latency measurements;
- `headless-watch start/status/stop/open` CLI commands, log size limits, and notification click-through.

## What it does

- Detects headless Chrome, Chromium, Edge, `chrome-headless-shell`, and Playwright-managed Chromium roots.
- Separates **launcher** (OpenCode, Codex, Claude Code) from **browser driver** (Playwright, Puppeteer) as independent observed facts.
- Reports CPU as `cores`, `% of one core`, and `% of machine` with the live online-CPU denominator.
- Reports `ΣRSS` as an upper-bound estimate and low-frequency `ΣPSS` with coverage and sample age.
- Keeps source snapshots, lifecycle events, metric history, alert evidence, and action audits in SQLite WAL.
- Provides a Vue 3 dashboard for Overview, Live Sessions, History, Alerts, and Settings.
- Allows on-demand screenshots and verified browser-subtree termination: Linux signals or an explicitly confirmed Windows force stop.

## Repository layout

```text
backend/         Rust/Tokio collector, Axum API, SQLite WAL storage
frontend/        Vue 3 + TypeScript dashboard (Vite)
tools/           Repository-level end-to-end Rust/Vue smoke test
docs/            Product requirements
install.sh       Builds and installs the user service
```

- Linux with `/proc`, or Windows 10/11 with the MSVC Rust toolchain
- Rust 1.85+ and Cargo (backend build)
- Node 20+ and pnpm 9+ (dashboard build; not needed at runtime)
- Chrome, Chromium, Edge, or `chrome-headless-shell` for browser detection
- Optional: `notify-send` for desktop notifications

No root or administrator privileges are required. Headless Watch only inspects and terminates processes owned by the current user. Windows source builds also require the Visual Studio C++ Build Tools and Windows SDK.

## Run from source

Build the dashboard once:

```bash
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
```

Build and run the backend:

```bash
cargo run --release --manifest-path backend/Cargo.toml -- \
  --web-dir frontend/dist \
  --data-dir "$HOME/.local/share/headless-watch/data"
```

Open **http://127.0.0.1:8765**. The server binds only to `127.0.0.1`.

On Windows PowerShell:

```powershell
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend build
cargo run --release --manifest-path backend/Cargo.toml -- --web-dir frontend/dist
```

The default Windows database directory is `%LOCALAPPDATA%\headless-watch\data`; use `--data-dir` for a separate development database.

## Install for the current user

Windows PowerShell (source build, or extracted Windows release archive):

```powershell
.\install.ps1
# Inspect the plan without changing files or login startup:
.\install.ps1 -DryRun
# Optional isolated install without login startup:
.\install.ps1 -InstallDir "$PWD\dist\local-install" -NoAutoStart -SkipStart
& "$env:LOCALAPPDATA\headless-watch\bin\headless-watch.exe" status
```

Windows installs to `%LOCALAPPDATA%\headless-watch` and registers a shortcut in the current user's Startup folder. `start` launches a hidden background monitor; `stop` verifies its PID, creation time, and executable before requesting graceful shutdown through a native event. Upgrades stop the verified monitor before replacing its executable. `-NoAutoStart` skips shortcut changes and `-SkipStart` skips immediate startup. No Windows service or elevated task is created.

```powershell
& "$env:LOCALAPPDATA\headless-watch\uninstall.ps1"             # preserve data and logs
& "$env:LOCALAPPDATA\headless-watch\uninstall.ps1" -RemoveData # remove data explicitly
```

See [Windows setup and limits](docs/WINDOWS.md). Linux retains its systemd user service:

```bash
bash install.sh
```

The installer:

1. builds the Vue dashboard if `frontend/dist` is missing,
2. builds the release Rust backend,
3. installs the binary to `~/.local/share/headless-watch/bin/headless-watch`,
4. installs the dashboard to `~/.local/share/headless-watch/web`,
6. installs and starts the `headless-watch` systemd **user** service.

It does not require sudo and does not delete the SQLite data directory.

```bash
systemctl --user status headless-watch
systemctl --user stop headless-watch
systemctl --user disable --now headless-watch
```

To uninstall:

```bash
systemctl --user disable --now headless-watch
rm -f ~/.config/systemd/user/headless-watch.service
rm -rf ~/.local/share/headless-watch
systemctl --user daemon-reload
```

Python v0.2 is no longer distributed. The Rust service is the only supported backend; its SQLite history lives in the configured `--data-dir`.

## Dashboard

- **Overview**: active sessions, CPU cores, system CPU share, suspected leftovers, three live charts (CPU cores, % machine, memory), session cards, recent activity, and a **Recently exited** list with click-through to full history.
- **Live Sessions**: card or table view, sort by CPU/memory/runtime/recent/risk, source and risk filters, instant search, multi-select batch stop in table view, and a right-side Inspector.
- **Inspector**: Overview evidence, observed source chain and verified process tree, browser/CDP tab list with on-demand screenshots, session events, copy diagnostic report, mute, Stop, and Force kill.
- **History**: persisted session lifecycles on a Gantt-like timeline, source evidence, launcher-exit markers, peak CPU and memory, event history, and action audits. Ended sessions are retained in SQLite even though they disappear from the live inventory.
- **Alerts**: pending, acknowledged, and muted sustained-CPU or source-exit alerts with evidence and acknowledgement. On desktops that support notification actions, clicking a Headless Watch notification opens the matching session Inspector.
- **Settings**: sampling intervals, CPU alert threshold and duration, leftover delay, notification mute, screenshot permission, retention, and export controls.

Keyboard shortcuts: `/` focuses session search, `Esc` closes the Inspector or a dialog, `?` opens the shortcut list.

## Metrics definitions

- **CPU (cores)**: sum of per-process CPU time deltas divided by elapsed time. Linux converts `utime + stime` using `CLK_TCK`; Windows converts native kernel/user time from 100 ns ticks. New child processes never import historic ticks.
- **CPU (one core)**: `cores × 100%`; a browser tree can exceed 100%.
- **CPU (machine)**: `cores / online CPUs × 100%`, using the current online CPU count.
- **ΣRSS**: arithmetic sum of resident memory for the browser root and its current descendants. This is an **upper bound** because Chrome shares pages.
- **ΣPSS**: Linux proportional set size from `smaps_rollup`, reported with sample age and `sampled/total` coverage. Windows does not expose the same measurement and reports PSS as unavailable. PSS sampling never blocks the one-second process scan.
- **Peak CPU**: highest observed `cores` value for the session in the current run.
- CPU first appears as `Calculating`; it requires two observations of the same process identity.

A browser that exits is marked stopped even if its parent has not yet reaped the zombie. A launcher that exits while the browser remains becomes a **suspected leftover** after the configured delay; this is an observation, not a claim that the browser is unwanted.

## Source evidence

- Attribution uses observed ancestry at discovery time, plus cgroup evidence on Linux when available.
- **Observed** means the ancestor chain was directly visible. **Inferred** uses cgroup evidence. **Unknown** is used when nothing can be proven.
- A browser already running before the monitor starts is marked `preexisting_at_start`; its origin is not reconstructed after the fact.
- Launcher and driver are separate fields. `Playwright` is never reported as the launcher when the observed launcher is `OpenCode`.
- The source chain is persisted immediately so it survives a launcher exit and a backend restart.

## CDP and screenshots

- CDP can be `tcp`, `pipe`, or `none`. `DevToolsActivePort` is read for `--remote-debugging-port=0`.
- Only an endpoint that belongs to the observed local browser process is used; the API never accepts an arbitrary CDP URL.
- Tab titles and URLs are shown with query parameters and credentials redacted.
- Screenshots are captured only when the user requests them, through an already-running local TCP endpoint. Captures are not stored by the backend.
- A browser using `--remote-debugging-pipe` can be fully monitored but cannot be screenshotted or attached to.

## Stop, Force kill, and audit

The dashboard requires explicit confirmation. Before signaling, the backend re-discovers the process tree and verifies boot ID, PID plus start time, same user (Linux UID / Windows SID), and current headless-browser evidence. It signals only the verified current browser subtree; the launcher, IDE, Node, and unrelated Chrome processes are never included.

- **Stop** on Linux sends `SIGTERM` to children first and the root last. Windows cannot reliably provide a graceful signal to a detached console/browser process; the UI offers an explicitly confirmed **Force stop**, backed by identity-verified `TerminateProcess`. A Windows API request for graceful stop is rejected without terminating a process.
- **Batch stop** works from the Live table: select up to 20 sessions, review the second confirmation, and each session is re-verified independently. Rejected sessions do not affect the others.
- On Linux, if the root survives, **Force kill** becomes available for a separately confirmed `SIGKILL`.
- A stale scan, reused PID, or changed browser evidence is rejected.
- Every attempt, rejection, target count, signal count, survivor count, and result is stored in the SQLite action audit.

## Alerts, notifications, and retention

- CPU alerts require a sustained threshold crossing (default `1.0 cores` for `30s`), never a single sample.
- A CPU alert is emitted once per episode, with a clear event when CPU returns below `threshold × clear ratio`.
- Global mute and per-session mute suppress desktop notifications while keeping alert events.
- Parent, launcher, and driver exits update the session status and event log but do not produce desktop notifications; only new browsers, suspected leftovers, sustained CPU alerts, and action failures notify.
- Linux desktop notifications use `notify-send` and contain only the browser name, PID, source summary, and metric values — no URLs or full commands. Windows reports desktop notifications as unavailable; alerts, evidence, and acknowledgement remain available in the dashboard.
- SQLite WAL stores raw one-second metrics for the configured raw window (default 24 hours), one-minute rollups for 7 days, and events, action audits, and ended-session metadata for the configured event window (default 30 days).

## API

All endpoints are loopback-only. `/api/v1/health` is public; everything else requires the random token embedded in the served dashboard.

```http
GET  /api/v1/health
GET  /api/v1/overview
GET  /api/v1/stream                       # SSE, full snapshots with revision ids
GET  /api/v1/sessions?status=active|ended
GET  /api/v1/sessions/{id}
GET  /api/v1/sessions/{id}/metrics?window=30m
GET  /api/v1/sessions/{id}/tabs
POST /api/v1/sessions/{id}/screenshot     # {tab_id}
POST /api/v1/sessions/{id}/terminate      # identity + mode + confirm
POST /api/v1/terminate                    # batch: sessions[] + mode + confirm
POST /api/v1/sessions/{id}/mute           # {minutes}
GET  /api/v1/events?since=&limit=&session_id=
GET  /api/v1/actions?limit=
GET  /api/v1/alerts?limit=
POST /api/v1/alerts/{id}/ack
POST /api/v1/alerts/{id}/mute             # {minutes}
GET  /api/v1/settings
PATCH /api/v1/settings
GET  /api/v1/export?format=json|csv&dataset=sessions|events|actions
```

## Security

- Dashboard and API bind to `127.0.0.1` only.
- A random 256-bit token is generated on every launch, embedded in the dashboard page, and never written to disk.
- `Host`, `Origin`, and `Sec-Fetch-Site` are validated for state-changing requests; no CORS headers are emitted.
- The dashboard is served with a strict Content Security Policy, `X-Frame-Options: DENY`, `nosniff`, and no referrer.
- Commands, URLs, paths, and secret switches are redacted before storage or display.
- Linux creates the SQLite data directory with `0700` permissions and the database file with `0600`. Windows uses the inherited ACLs of the current user's LocalAppData directory; choose an appropriately private directory when overriding `--data-dir`.
- This is not a security boundary against a malicious process running as the same user. For guaranteed cleanup, isolate each agent in a systemd service or cgroup.

## Themes

The original dark **Observatory** interface is the default. A second **Terminal** theme is available from the top-bar switch; it was designed with the `design-taste-frontend` skill and uses a 64px icon rail, a live radar signal field, a dense signal log with CPU/event traces, a single wide history scope, Public Sans with IBM Plex Mono figures and 1px accent selection lines. The choice persists in `localStorage`; use `?theme=terminal` to force it for a screenshot. See [`docs/THEMES.md`](docs/THEMES.md) for the direction, face and verification results.

## Upgrades and data

The SQLite schema is versioned. Headless Watch refuses to open a database written by a newer build and keeps restart state within the supported schema version; back up `--data-dir` before downgrading.

## Logging

When installed as a user service, logs go to journald and are rate-limited by the unit (`LogRateLimitIntervalSec=30s`, `LogRateLimitBurst=200`):

```bash
journalctl --user -u headless-watch -f
```

`headless-watch status` reports the running version and active session count.

On Windows, installed background output is written to `%LOCALAPPDATA%\headless-watch\logs\headless-watch.log`. At the next start, logs larger than 2 MiB rotate to one `headless-watch.previous.log`; a continuously running process does not rotate its log. Windows login startup does not restart the monitor automatically after a crash.

## Commands and options

```text
headless-watch start           Start the installed per-user background monitor
headless-watch status          Show service or local HTTP health (--port)
headless-watch stop            Stop the installed per-user background monitor
headless-watch open            Open the dashboard in the desktop browser (--port)

--port PORT                    Loopback HTTP port (default: 8765)
--interval SECONDS             Process scan interval, 0.5-10 (default: 1)
--cpu-alert-cores CORES        Sustained CPU threshold in logical cores (default: 1.0)
--cpu-alert-duration SECONDS   Seconds above threshold before alerting (default: 30)
--leftover-after SECONDS       Delay before flagging a suspected leftover (default: 60)
--pss-interval SECONDS         PSS sampling interval, 5-300 (default: 15)
--cdp-interval SECONDS         Existing CDP polling interval, 2-120 (default: 5)
--no-notify                    Disable desktop notifications for this run
--verbose                      Enable debug logging (requests, actions, scans)
--data-dir PATH                SQLite and history directory
--web-dir PATH                 Built Vue dashboard directory
--version                      Print the version
```

CLI values apply to the current run. Values saved from the Settings page persist in SQLite.

## Performance benchmark

```bash
python3 tools/benchmark.py --seconds 60
```

It starts a private server, warms up, then reports backend CPU and RSS. On the current Ubuntu test machine it measures **~1.0% of one logical core** and **~11.5 MiB RSS** with the default 1s scan and existing browser sessions. To keep idle overhead low, PSS sampling backs off to 30s when no dashboard is connected; with a dashboard connected it uses the configured interval (default 15s). Use `--max-cpu 1.0` to turn the benchmark into a regression gate.

## Tests

Rust unit and API tests:

```bash
cargo test --all-targets --manifest-path backend/Cargo.toml
```

Full Rust + Vue end-to-end smoke test (private server, target and observer Chrome, rendered UI, high-CPU alert, Inspector, screenshot endpoint, UI-confirmed stop, action audit, and history across a backend restart):

```bash
python3 tools/rust_dashboard_smoke.py
```

Linux-only acceptance checks (Force kill, real crash convergence, OpenCode → playwright-mcp → Chrome lifecycle, 101-process stress, cross-site API attempts, port conflict/no-orphan):

```bash
python3 tools/linux_acceptance.py
```

The browser tests launch their own temporary Chrome instances with isolated profiles. Existing browsers and user profiles are never touched.

## Packaging

Create a prebuilt tarball (binary, dashboard assets, installer, uninstaller, docs):

```bash
bash tools/package-release.sh
```

The archive is written to `dist/` together with a SHA-256 checksum. Its `install.sh` installs the prebuilt files directly and does not require Cargo or pnpm. Uninstall with the installed `uninstall.sh` or the repository copy:

```bash
bash uninstall.sh            # remove service, binaries, and data
bash uninstall.sh --keep-data
```

Create the equivalent Windows ZIP and SHA-256 checksum with `powershell -File tools/package-release.ps1`. The archive includes a prebuilt `.exe`, dashboard assets, `install.ps1`, and `uninstall.ps1`; installation does not require Cargo or pnpm.

## Known limitations

- No tray icon in v0.5; the dashboard is the system entry point.
- Windows desktop toast notifications and proportional memory (PSS) are unavailable; the dashboard retains alerts and working-set memory metrics.
- The dashboard is a single-user local tool; there is no remote authentication model.
- No continuous CDP screencast, page navigation, JavaScript execution, or input takeover.
- macOS is not supported.
- Linux-specific acceptance and benchmark scripts under `tools/` still require Linux. Windows uses native Rust tests and the setup workflow described in `docs/WINDOWS.md`.
