# Themes

The original dark interface remains the default. **Terminal** is the optional second theme, built with the `design-taste-frontend` (tasteskill v2) app-screen process.

## Observatory (default)

The original dark console: deep-blue ground, mint/cyan telemetry, session cards and the existing inspector/dialog styling. No existing component or class was removed to add the new theme.

## Terminal

- **Direction**: `data-terminal`, seed `744432032` (drawn 2026-10-22), extended into a custom operator console. Chosen because this is a long-session monitoring surface, not a marketing page.
- **Move**: a 64px icon rail, a radar signal field, a dense signal log with an embedded CPU trace and event trace, and a single wide signal-history scope. Every selection is a 1px accent line; figures use tabular monospace; there are no cards-in-a-grid ornaments.
- **Face**: Public Sans for interface text and IBM Plex Mono for figures, IDs and timestamps. Fonts are self-hosted under `frontend/src/assets/fonts/` (licences included).
- **Data hues**: CPU is white ink, machine share is blue (`#4a8ad6`), memory is blue/grey; red is reserved for alerts and negative status. Chrome stays neutral.
- **Signature elements**: the radar signal field (distance = CPU load, closer centre = higher; size = process count; angle = session identity; colour = launcher), the signal log + live event trace, the CPU trace strip, and the oversized telemetry readout strip. The sweep runs at 1.8s per revolution with a 60° trail. Radar/monitoring views are canvas; the CPU strip is real SVG data.
- **Screens covered**: overview, live sessions/list, history, alerts, settings, and the session inspector/dialogs.

## Switching

Use the **Observatory / Terminal** control in the top bar. The choice is stored in `localStorage` under `hw-theme`. For screenshots or the design inspector, force a theme with:

```text
http://127.0.0.1:8765/?theme=terminal
```

## Verification

- `design-taste-frontend` `inspect.mjs --app` at 1440, 768 and 390: **no FAILs**. One WARN remains for an empty band in the signal log when only one or two sessions exist; this is the honest empty-log state, not hidden data.
- Default Observatory smoke test passes, including Inspector, deep links, screenshots, alerts, stop flow and restart.

`lint.mjs --app` on the whole frontend still reports the intentional patterns of the original dark theme. The Terminal experience deliberately departs from the skill's quiet-app defaults; it was checked for layout and rendering in a real browser, with one accepted density warning. The new theme is isolated in `frontend/src/theme-terminal.css` and `TerminalOverview.vue`.
