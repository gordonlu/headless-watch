# Headless Watch

A local dashboard and history service for headless Chrome/Chromium browsers on Linux. It observes `/proc` and existing Chrome DevTools endpoints without modifying, restarting, or attaching input to the tools that launched the browser.

**v0.5** replaces the single-file Python monitor with a Rust/Tokio collector, Axum HTTP + SSE API, SQLite WAL history, and a Vue 3 dashboard. The Python v0.2 implementation was removed from the main branch after migration and is preserved under the `v0.2-python` git tag.

## Status

v0.5 is complete and v1.0 work is in progress. The remaining release gates are tracked in [`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md); the PRD stays in `docs/` until every v1.0 item is checked.

- [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) — modules, data flow, storage schema, failure boundaries.
- [`docs/SECURITY.md`](docs/SECURITY.md) — threat model, controls, and explicit limits.
- [`docs/RELEASE-CHECKLIST.md`](docs/RELEASE-CHECKLIST.md) — acceptance criteria status.
- [`docs/Headless-Watch-PRD-v1.0.md`](docs/Headless-Watch-PRD-v1.0.md) — original product requirements.

## Platform support

**Linux only.** The current implementation is built around Linux process observability and does not yet have a platform abstraction layer:

- `/proc` and `/proc/<pid>/smaps_rollup` for process identity, CPU ticks, and PSS.
- `/sys/devices/system/cpu/online` for the machine CPU denominator.
- systemd **user** services for installation and auto-start.
- `notify-send` for desktop notifications.
- XDG data paths (`~/.local/share/headless-watch`).

Tested on Ubuntu (Wayland session, 16 online CPUs, Chrome 155). Other systemd-based Linux distributions are expected to work but are not yet in the test matrix. Windows and macOS are Post-v1 per the PRD; adding them requires a platform abstraction (`ProcessSource` / `ServiceManager` / `Notifier`) plus a real-OS CI matrix.

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
- Allows on-demand screenshots and a verified `SIGTERM`/`SIGKILL` only for the currently verified browser subtree.

## Repository layout

```text
backend/         Rust/Tokio collector, Axum API, SQLite WAL storage
frontend/        Vue 3 + TypeScript dashboard (Vite)
tools/           Repository-level end-to-end Rust/Vue smoke test
docs/            Product requirements
install.sh       Builds and installs the user service
```

- Linux with `/proc`
- Rust 1.85+ and Cargo (backend build)
- Node 20+ and pnpm 9+ (dashboard build; not needed at runtime)
- Chrome, Chromium, Edge, or `chrome-headless-shell` for browser detection
- Optional: `notify-send` for desktop notifications

No root privileges are required. Headless Watch only inspects and terminates processes owned by the current user.

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

## Install as a user service

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

To inspect or restore the removed Python v0.2 implementation:

```bash
git checkout v0.2-python -- legacy-python
```

## Dashboard

- **Overview**: active sessions, CPU cores, system CPU share, suspected leftovers, three live charts (CPU cores, % machine, memory), session cards, recent activity, and a **Recently exited** list with click-through to full history.
- **Live Sessions**: card or table view, sort by CPU/memory/runtime/recent/risk, source and risk filters, instant search, multi-select batch stop in table view, and a right-side Inspector.
- **Inspector**: Overview evidence, observed source chain and verified process tree, browser/CDP tab list with on-demand screenshots, session events, copy diagnostic report, mute, Stop, and Force kill.
- **History**: persisted session lifecycles on a Gantt-like timeline, source evidence, launcher-exit markers, peak CPU and memory, event history, and action audits. Ended sessions are retained in SQLite even though they disappear from the live inventory.
- **Alerts**: pending, acknowledged, and muted sustained-CPU or source-exit alerts with evidence and acknowledgement.
- **Settings**: sampling intervals, CPU alert threshold and duration, leftover delay, notification mute, screenshot permission, retention, and export controls.

Keyboard shortcuts: `/` focuses session search, `Esc` closes the Inspector or a dialog, `?` opens the shortcut list.

## Metrics definitions

- **CPU (cores)**: sum of per-process `Δ(utime + stime) / elapsed / CLK_TCK`. New child processes never import historic ticks.
- **CPU (one core)**: `cores × 100%`; a browser tree can exceed 100%.
- **CPU (machine)**: `cores / online CPUs × 100%`, using the current online CPU count.
- **ΣRSS**: arithmetic sum of resident memory for the browser root and its current descendants. This is an **upper bound** because Chrome shares pages.
- **ΣPSS**: sampled proportional set size from `smaps_rollup`, reported with sample age and `sampled/total` coverage. PSS sampling never blocks the one-second process scan.
- **Peak CPU**: highest observed `cores` value for the session in the current run.
- CPU first appears as `Calculating`; it requires two observations of the same process identity.

A browser that exits is marked stopped even if its parent has not yet reaped the zombie. A launcher that exits while the browser remains becomes a **suspected leftover** after the configured delay; this is an observation, not a claim that the browser is unwanted.

## Source evidence

- Attribution uses the observed ancestry and cgroup at discovery time.
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

The dashboard requires explicit confirmation. Before signaling, the backend re-discovers the process tree and verifies boot ID, PID plus start time, same UID, and current headless-browser evidence. It signals only the verified current browser subtree; the launcher, IDE, Node, and unrelated Chrome processes are never included.

- **Stop** sends `SIGTERM` to children first and the root last.
- **Batch stop** works from the Live table: select up to 20 sessions, review the second confirmation, and each session is re-verified independently. Rejected sessions do not affect the others.
- If the root survives, **Force kill** becomes available for a separately confirmed `SIGKILL`.
- A stale scan, reused PID, or changed browser evidence is rejected.
- Every attempt, rejection, target count, signal count, survivor count, and result is stored in the SQLite action audit.

## Alerts, notifications, and retention

- CPU alerts require a sustained threshold crossing (default `1.0 cores` for `30s`), never a single sample.
- A CPU alert is emitted once per episode, with a clear event when CPU returns below `threshold × clear ratio`.
- Global mute and per-session mute suppress desktop notifications while keeping alert events.
- Desktop notifications use `notify-send` and contain only the browser name, PID, source summary, and metric values — no URLs or full commands.
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
- The SQLite data directory is created with `0700` permissions and the database file with `0600`.
- This is not a security boundary against a malicious process running as the same user. For guaranteed cleanup, isolate each agent in a systemd service or cgroup.

## Options

```text
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

## Tests

Rust unit and API tests:

```bash
cargo test --all-targets --manifest-path backend/Cargo.toml
```

Full Rust + Vue end-to-end smoke test (private server, target and observer Chrome, rendered UI, high-CPU alert, Inspector, screenshot endpoint, UI-confirmed stop, action audit, and history across a backend restart):

```bash
python3 tools/rust_dashboard_smoke.py
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

## Known limitations

- No tray icon in v0.5; the dashboard is the system entry point.
- Desktop notifications do not yet deep-link into the relevant Inspector; the Alerts page contains the same evidence.
- The dashboard is a single-user local tool; there is no remote authentication model.
- No continuous CDP screencast, page navigation, JavaScript execution, or input takeover.
- Windows and macOS are not supported.
- A packaged standalone Linux binary is planned for v1.0; v0.5 builds with Cargo and pnpm.
