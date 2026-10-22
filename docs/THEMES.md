# Themes

The refined dark **Observatory** interface is the default. **Terminal** remains the optional second theme.

## Observatory (default)

Charcoal surfaces (`#101216`, `#171a20`), fine grey borders, teal activity and blue secondary telemetry. Public Sans UI text and IBM Plex Mono figures are self-hosted. A 224px sidebar, four key metrics, three telemetry plots, horizontal session rows, and a quieter activity log retain the existing navigation and workflows. On phones all five navigation destinations remain visible, metrics form two columns, and charts and session details stack. Keyboard focus and reduced-motion support are retained.

The styling lives in `frontend/src/theme-observatory.css`, scoped away from Terminal. Both themes use backend-reported platform capabilities: Windows shows Working Set memory, disables unsupported PSS sampling, and presents an explicitly confirmed Force stop action rather than a SIGTERM label.

## Terminal

- **Direction**: `data-terminal`, seed `744432032` (drawn 2026-10-22), extended into a custom operator console. Chosen because this is a long-session monitoring surface, not a marketing page.
- **Move**: a 64px icon rail, a radar signal field, a dense signal log with an embedded CPU trace and event trace, and a single wide signal-history scope. Every selection is a 1px accent line; figures use tabular monospace; there are no cards-in-a-grid ornaments.
- **Face**: Public Sans for interface text and IBM Plex Mono for figures, IDs and timestamps. Fonts are self-hosted under `frontend/src/assets/fonts/` (licences included).
- **Data hues**: CPU is white ink, machine share is blue (`#4a8ad6`), memory is blue/grey; red is reserved for alerts and negative status. Chrome stays neutral.
- **Signature elements**: the radar signal field (distance = absolute CPU load (1 busy core sits at the centre); size = process count; angle = session identity; colour = launcher), the signal log + live event trace, the CPU trace strip, and the oversized telemetry readout strip. The sweep runs at 1.8s per revolution with a 60° trail. Radar/monitoring views are canvas; the CPU strip is real SVG data.
- **Screens covered**: overview, live sessions/list, history, alerts, settings, and the session inspector/dialogs.

## Switching

Use the **Observatory / Terminal** control in the top bar. The choice is stored in `localStorage` under `hw-theme`. For screenshots or the design inspector, force a theme with:

```text
http://127.0.0.1:8765/?theme=terminal
```

## Verification

The Windows refinement was checked in real Chrome with Playwright at 1440×1000, 768×1024, and 390×844: navigation, session search, table mode, Inspector, theme switching, Windows PSS controls, and horizontal overflow. The visual reference and rendered screen were inspected together for sidebar layout, palette, typography, metric hierarchy, chart framing, and session controls. Data comes from the native Windows collector; reference figures are illustrative. The result deliberately retains source-confidence evidence, per-core CPU units, and an event timeline instead of copying illustrative reference data or replacing brand marks with third-party artwork.

Earlier Terminal verification:

- `design-taste-frontend` `inspect.mjs --app` at 1440, 768 and 390: **no FAILs**. One WARN remains for an empty band in the signal log when only one or two sessions exist; this is the honest empty-log state, not hidden data.
- Default Observatory smoke test passes, including Inspector, deep links, screenshots, alerts, stop flow and restart.

`lint.mjs --app` on the whole frontend still reports the intentional patterns of the original dark theme. The Terminal experience deliberately departs from the skill's quiet-app defaults; it was checked for layout and rendering in a real browser, with one accepted density warning. The new theme is isolated in `frontend/src/theme-terminal.css` and `TerminalOverview.vue`.
