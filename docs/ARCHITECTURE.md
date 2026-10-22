# Architecture

Headless Watch is a local observability service for headless Chromium browsers. It reads `/proc` and existing Chrome DevTools endpoints; it never restarts, patches, or attaches input to the tools that launched a browser.

## Components

```text
/proc (same UID)                     existing local CDP endpoint
      │                                        │
      ▼                                        ▼
┌─────────────────────────────────────────────────────────────┐
│ backend/ · Rust + Tokio                                     │
│  procfs        /proc scanning, identity, PSS                │
│  browser       headless root detection, CDP endpoint        │
│  attribution   observed ancestry, launcher/driver evidence  │
│  tracker       sessions, per-PID CPU deltas, lifecycle      │
│  rules         sustained CPU alerts, mute, cooldown         │
│  actions       verified SIGTERM/SIGKILL and batch cleanup   │
│  storage       SQLite WAL, retention, export                │
│  server        Axum HTTP + SSE, token/Host/Origin checks    │
└───────────────┬─────────────────────────────┬───────────────┘
                │                             │
          SQLite WAL                    Axum HTTP + SSE
                │                             │
      sessions / metrics / events          Vue 3 SPA
      actions / settings / rollups        frontend/
```

## Backend modules

- `procfs.rs`: `/proc/<pid>/stat`, `cmdline`, `exe`, `status`, `cgroup`, `smaps_rollup`; online CPU count; boot ID; descendant discovery.
- `browser.rs`: executable allow-list, `--headless` evidence, `--type=` child exclusion, `DevToolsActivePort` parsing.
- `attribution.rs`: observed ancestor chain, launcher/driver token classification, cgroup inference, confidence levels.
- `tracker.rs`: browser session identity (`boot_id + pid + start_ticks`), per-PID CPU deltas, RSS/PSS aggregation, lifecycle events, alerts, session views.
- `storage.rs`: SQLite WAL schema, session/event/action persistence, one-minute metric rollups, retention cleanup, export queries.
- `cdp.rs`: local `/json/list` probing and on-demand screenshot capture over an existing TCP endpoint.
- `actions.rs`: re-verification of boot ID, PID + start time, UID, and browser evidence before signaling the verified subtree.
- `server.rs`: `/api/v1` routes, SSE snapshots, static Vue assets, CSP and request validation.
- `core.rs`: shared state, settings, scan loop, PSS/CDP side work, audit records.

## Data flow

1. The scanner reads `/proc` every configured interval and builds a snapshot.
2. Headless browser roots are identified; each root becomes a session keyed by boot ID, PID, and start time.
3. Per-process CPU ticks are diffed against the previous scan. New processes never import historic ticks.
4. Source evidence is captured immediately and persisted, so it survives a launcher exit or a backend restart.
5. Active views are serialized into the SSE stream; ended sessions are persisted and exposed through the History and Recently exited views.
6. PSS and CDP polling run on a slower side loop so a slow `smaps_rollup` read or an unreachable endpoint never delays process discovery.
7. Stop/kill requests re-discover the process tree and verify identity before sending signals; every attempt is audited.

## Storage

SQLite runs in WAL mode with the database file at `0600` and the data directory at `0700`.

| Table | Purpose | Default retention |
|---|---|---|
| `sessions` | session metadata, source evidence, final view | 30 days after exit |
| `metric_samples` | one-second CPU/RSS/PSS samples | configured raw window (24h) |
| `metric_rollups` | one-minute aggregates | 7 days |
| `events` | lifecycle, alert, and scan events | configured event window (30 days) |
| `actions` | verified stop/kill audits | configured event window (30 days) |
| `settings` | persisted user settings | until changed |

## Frontend

The Vue 3 SPA talks to `/api/v1` with the token embedded in the served page. It keeps one EventSource connection for snapshots and falls back to periodic fetches when the stream is unavailable. The Inspector, batch confirmation, and screenshot dialogs are local-only and never store page content on the server.

## Failure boundaries

- A failed `/proc` scan marks sessions stale instead of ending them.
- A PID reused by another process never matches the stored start time and cannot be terminated.
- A dead browser root is treated as exited even when its parent has not reaped the zombie.
- An unreachable CDP endpoint is reported as `endpoint_unreachable`; process monitoring continues.
- A backend restart reconciles previously open sessions and preserves history and active session identity.
