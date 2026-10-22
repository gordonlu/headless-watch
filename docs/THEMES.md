# Themes

The original dark interface remains the default. The **Framed** theme is an optional light alternative, built with the `design-taste-frontend` (tasteskill v2) app-screen process.

## Observatory (default)

The original dark console: deep-blue ground, mint/cyan telemetry, compact session cards and the existing inspector/dialog styling. No existing component or class was removed to add the new theme.

## Framed

- **Direction**: `framed-sheet`, seed `1848647026` (drawn 2026-10-22).
- **Move**: a tinted L-frame; the working canvas is a white sheet with a 1px edge, 16px corners, and no shadow. The page uses one gap value and a single set of outer margins.
- **Face**: IBM Plex Sans for the interface, IBM Plex Mono for IDs and measured figures. Fonts are self-hosted under `frontend/src/assets/fonts/` (SIL OFL, licence included).
- **Data hue**: the main charts use one teal/blue family (`#1b6c80`, `#2f70bb`) with an amber PSS comparison (`#a86f0c`). Chrome stays neutral.
- **Signature elements**: the four-cell KPI strip split by dividers, the three-column CPU/machine/memory telemetry row, and the provenance/session sparkline that carries the product's live identity.
- **Screens covered**: overview, live sessions/list, history, alerts, settings, and the session inspector/dialogs.

## Switching

Use the **Observatory / Framed** control in the top bar. The choice is stored in `localStorage` under `hw-theme`. For screenshots or the design inspector, force a theme with:

```text
http://127.0.0.1:8765/?theme=framed
```

## Verification

- `design-taste-frontend` `inspect.mjs --app` at 1440, 768 and 390: **0 findings** for `?theme=framed`.
- Rust/Vue smoke test: default theme continues to pass, including Inspector, deep links, screenshots, alerts, stop flow and restart.

`lint.mjs --app` on the whole frontend still reports the intentional patterns of the original dark theme (gradients, eyebrows, monospace telemetry and shadows). Those are scoped to the default theme and are not present in the Framed overrides; the new theme is isolated in `frontend/src/theme-framed.css`.
