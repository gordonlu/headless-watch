# Security model

Headless Watch observes processes owned by the current user and can signal a verified headless browser subtree. It is a local single-user tool, not a multi-user service and not a security boundary against a malicious process running as the same user.

## Controls

- **Loopback only**: the HTTP server binds to `127.0.0.1`; there is no remote access mode.
- **Random token**: a 256-bit token is generated at launch, embedded in the dashboard page, and never written to disk.
- **Host validation**: requests must use `127.0.0.1:<port>` or `localhost:<port>`; other Host values are rejected to reduce DNS-rebinding risk.
- **Origin and fetch metadata**: state-changing requests must present a same-origin `Origin` when the browser sends one and a `Sec-Fetch-Site` value of `same-origin` or `none`.
- **No CORS**: cross-origin reads and writes are not enabled.
- **CSP**: the dashboard is served with `default-src 'none'`, `script-src 'self'`, `style-src 'self'`, `frame-ancestors 'none'`, and no inline script or style.
- **Content type**: JSON bodies must use `application/json`; bodies are limited to 4 KiB.
- **Redaction**: secret switches, URL query strings, credentials, control characters, and home paths are removed before data crosses the API boundary or is stored.
- **Database permissions**: WAL mode; Linux data directory `0700` and database file `0600`. Windows inherits the current user's LocalAppData ACLs; an overridden data directory must have suitable ACLs.
- **CDP SSRF prevention**: the backend only connects to a local WebSocket URL that was discovered from the observed browser process and matches the session port and `/devtools/page/` path. The API never accepts an arbitrary CDP address.
- **Verified termination**: boot ID, PID plus start time, same user, and current headless-browser evidence are re-checked immediately before signaling. Linux uses UID and SIGTERM/SIGKILL. Windows verifies token SID, precise creation FILETIME, and ancestry through retained process handles before an explicitly confirmed force stop; graceful browser stop is unavailable. Only the verified current subtree is targeted. Parents, IDEs, Node, and unrelated Chrome processes are excluded.
- **Audit**: every stop, kill, batch item, and rejection is recorded with target counts and result in SQLite.
- **Notifications**: desktop notifications contain browser name, PID, source summary, and metric values only; they never include URLs or full commands.
- **Windows monitor lifecycle**: `stop` verifies monitor PID, creation FILETIME, and canonical executable path before setting its random native shutdown event. No executable-name process kill is used. Windows installation uses only the current user's Startup shortcut and preserves data by default on uninstall.

## Out of scope

- A malicious process running as the same user can read the same `/proc` data and could attempt to interfere with the dashboard or its data directory.
- The dashboard has no remote authentication model; do not expose the port through a proxy, tunnel, or reverse proxy.
- Screenshots can reveal logged-in page content. Use an isolated browser profile and avoid sharing captures.
- `--remote-debugging-pipe` browsers can be monitored but not attached to or screenshotted.
- The service does not upload telemetry and does not collect browser cookies, passwords, or request bodies.
