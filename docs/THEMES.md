# Themes

The original dark interface remains the default. **Terminal** is the optional second theme, built with the `design-taste-frontend` (tasteskill v2) app-screen process.

## Observatory (default)

The original dark console: deep-blue ground, mint/cyan telemetry, session cards and the existing inspector/dialog styling. No existing component or class was removed to add the new theme.

## Terminal

- **Direction**: `data-terminal`, seed `744432032` (drawn 2026-10-22). Chosen because this is a long-session monitoring surface, not a marketing page.
- **Move**: every selection is a 1px accent line on the nearest edge; figures use tabular monospace; cards are darker than the ground; there are no filled selection pills or decorative glows.
- **Face**: Public Sans for interface text and IBM Plex Mono for figures, IDs and timestamps. Fonts are self-hosted under `frontend/src/assets/fonts/` (licences included).
- **Data hues**: CPU is white ink, machine share is blue (`#4a8ad6`), memory is blue/grey; red is reserved for alerts and negative status. Chrome stays neutral.
- **Signature elements**: a 64px icon rail, a full-width signal deck with three stacked oscilloscope-style charts, the four-cell KPI strip, and dense full-width session rows with a provenance sparkline.
- **Screens covered**: overview, live sessions/list, history, alerts, settings, and the session inspector/dialogs.

## Switching

Use the **Observatory / Terminal** control in the top bar. The choice is stored in `localStorage` under `hw-theme`. For screenshots or the design inspector, force a theme with:

```text
http://127.0.0.1:8765/?theme=terminal
```

## Verification

- `design-taste-frontend` `inspect.mjs --app` at 1440, 768 and 390: **0 findings** for `?theme=terminal`.
- Default Observatory smoke test passes, including Inspector, deep links, screenshots, alerts, stop flow and restart.

`lint.mjs --app` on the whole frontend still reports the intentional patterns of the original dark theme (gradients, eyebrows, monospace telemetry and shadows). Those are scoped to the default theme; the Terminal overrides live in `frontend/src/theme-terminal.css` and were checked through the real-browser app inspector.
