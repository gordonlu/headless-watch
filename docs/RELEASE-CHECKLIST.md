# Release checklist

Tracks the PRD acceptance criteria. The PRD can be deleted only when every v1.0 gate below is checked.

## v0.2 gate (Milestone A)

| ID | Status | Evidence |
|---|---|---|
| AT-01 GUI Chrome is not detected | Done | `browser.rs` tests, dashboard smoke |
| AT-02 headless browser appears within 2s | Done | 1s scanner; Rust smoke and legacy tests |
| AT-03 CPU shows cores / one-core / machine | Done | `metrics` tests and dashboard smoke |
| AT-04 new renderer does not import historic ticks | Done | tracker and legacy tests |
| AT-05 RSS is an upper bound; PSS has coverage and age | Done | tracker, storage, dashboard smoke |
| AT-09 pipe-only browsers are monitored without CDP | Done | browser/tracker tests, legacy integration |
| AT-11 stale identity is rejected | Done | Rust smoke stale-identity rejection, legacy tests |

## v0.5 gate (Milestone B)

| ID | Status | Evidence |
|---|---|---|
| AT-06 launcher/parent exit keeps source history | Done | tracker tests for launcher exit, parent exit, suspected leftover; SQLite persistence |
| AT-07 pre-existing browser reports Unknown origin | Done | tracker pre-existing test |
| AT-08 CDP tabs and on-demand screenshot | Done | Rust smoke screenshot endpoint and Browser tab |
| AT-12 history survives restart; UI reconnects | Done | Rust smoke restart, active rehydration, SSE reconnect |
| AT-14 sustained CPU alert, mute, dedupe | Done | real high-CPU smoke, unit tests for cooldown and mute |
| AT-15 Wayland/no-display operation | Done | server verified with `DISPLAY` and `WAYLAND_DISPLAY` unset |

## v1.0 gate (Milestone C)

| ID | Status | Remaining work |
|---|---|---|
| AT-10 Stop, Force kill, batch, full audit | Done | UI stop, batch stop, SIGTERM-ignoring root force kill, stale-identity rejection, and audit rows verified end-to-end. |
| AT-11 PID reuse rejection | Done | A request with a mismatched start time is rejected while the process stays alive; the same start-time guard covers PID reuse. |
| AT-13 malicious local page cannot call the API | Done | A real page on another loopback origin is blocked for both overview and terminate. |
| AT-16 browser crash converges in UI | Done | Real headless Chrome `SIGKILL`; session leaves the live list and appears in ended history and Recently exited. |
| AT-17 port conflict and no orphan monitor | Done | Second server exits with a readable bind error; the first server stops cleanly with no orphan process. |
| AT-18 100 browser child processes stay responsive | Done | Real 101-process tree: overview max ~1-3ms, backend RSS ~13 MiB. |

## Other v1.0 items

- [ ] Measure performance targets: RSS is ~11-13 MiB; CPU measured at ~1.0% of one logical core in the acceptance sample after batching SQLite writes, lazy `/proc` reads, and session persist throttling. Record a longer repeatable benchmark before release.
- [x] Add `headless-watch start/status/stop/open` CLI commands (PRD FR-09).
- [x] Notification click-through opens the related session Inspector (PRD FR-07); depends on desktop notification action support.
- [x] Service log rate limiting via systemd journald plus `status` version/health reporting (PRD FR-09).
- [ ] Update/upgrade state restoration beyond the current SQLite schema version.
- [ ] Run one full real OpenCode + Playwright MCP lifecycle against the Rust backend: normal exit, forced interrupt, agent exit with browser alive, CDP available and unavailable.
- [ ] Repeatable performance benchmark with a target under 1% of one logical core (current samples are ~1.0-1.4%).
- [ ] Publish the release archive and tag the version.
- [ ] Optional, non-blocking: tray icon.

## Completed v1.0 foundations

- [x] Batch cleanup with explicit multi-select and second confirmation (`POST /api/v1/terminate`).
- [x] Copy diagnostic report and per-session mute.
- [x] Prebuilt release tarball (`tools/package-release.sh`).
- [x] Uninstaller with `--keep-data`.
- [x] LICENSE and expanded README, architecture, and security docs.
