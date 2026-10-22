#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="$HOME/.local/share/headless-watch"
UNIT_DIR="$HOME/.config/systemd/user"
BIN_DIR="$APP_DIR/bin"
WEB_DIR="$APP_DIR/web"
DATA_DIR="$APP_DIR/data"

if [ -x "$HERE/bin/headless-watch" ] && [ -f "$HERE/web/index.html" ]; then
  BIN_SRC="$HERE/bin/headless-watch"
  WEB_SRC="$HERE/web"
else
  if ! command -v cargo >/dev/null 2>&1; then
    echo "install.sh: cargo is required to build the Rust backend." >&2
    exit 1
  fi
  if ! command -v pnpm >/dev/null 2>&1 && [ ! -f "$HERE/frontend/dist/index.html" ]; then
    echo "install.sh: pnpm is required to build the Vue dashboard (or provide frontend/dist)." >&2
    exit 1
  fi
  echo "Building dashboard assets…"
  if [ ! -f "$HERE/frontend/dist/index.html" ]; then
    pnpm --dir "$HERE/frontend" install --frozen-lockfile
    pnpm --dir "$HERE/frontend" build
  fi
  echo "Building Rust backend (release)…"
  cargo build --release --manifest-path "$HERE/backend/Cargo.toml"
  BIN_SRC="$HERE/backend/target/release/headless-watch"
  WEB_SRC="$HERE/frontend/dist"
fi

VERSION="$("$BIN_SRC" --version 2>/dev/null | awk '{print $2}' || echo unknown)"
mkdir -p "$APP_DIR" "$UNIT_DIR" "$BIN_DIR" "$DATA_DIR"
install -m 0755 "$BIN_SRC" "$BIN_DIR/headless-watch"
rm -rf "$WEB_DIR"
cp -a "$WEB_SRC" "$WEB_DIR"
if [ -f "$HERE/uninstall.sh" ]; then
  install -m 0755 "$HERE/uninstall.sh" "$APP_DIR/uninstall.sh"
fi

cat > "$UNIT_DIR/headless-watch.service" <<'UNIT'
[Unit]
Description=Headless Watch - local Chromium observability (Rust + SQLite WAL)
Documentation=https://chromedevtools.github.io/devtools-protocol/
After=graphical-session.target

[Service]
Type=simple
WorkingDirectory=%h/.local/share/headless-watch
ExecStart=%h/.local/share/headless-watch/bin/headless-watch --port 8765 --web-dir %h/.local/share/headless-watch/web --data-dir %h/.local/share/headless-watch/data
Restart=on-failure
RestartSec=3
Environment=RUST_LOG=warn
LogRateLimitIntervalSec=30s
LogRateLimitBurst=200
NoNewPrivileges=true
PrivateTmp=true
KillSignal=SIGTERM
TimeoutStopSec=10

[Install]
WantedBy=default.target
UNIT

systemctl --user daemon-reload
systemctl --user enable --now headless-watch.service
echo
printf 'Headless Watch %s installed as a user service.\nDashboard: http://127.0.0.1:8765\nStatus: systemctl --user status headless-watch\nStop: systemctl --user disable --now headless-watch\nUninstall: bash %s/uninstall.sh [--keep-data]\nData: %s\n' "$VERSION" "$APP_DIR" "$DATA_DIR"
