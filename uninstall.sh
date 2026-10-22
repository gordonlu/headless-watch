#!/usr/bin/env bash
set -euo pipefail
APP_DIR="$HOME/.local/share/headless-watch"
UNIT_DIR="$HOME/.config/systemd/user"
KEEP_DATA=0
if [ "${1:-}" = "--keep-data" ]; then
  KEEP_DATA=1
fi

systemctl --user disable --now headless-watch.service >/dev/null 2>&1 || true
rm -f "$UNIT_DIR/headless-watch.service"
systemctl --user daemon-reload >/dev/null 2>&1 || true

if [ "$KEEP_DATA" = "1" ]; then
  rm -rf "$APP_DIR/bin" "$APP_DIR/web" "$APP_DIR/python-legacy"
  echo "Removed the Headless Watch service and binaries. SQLite data kept in $APP_DIR/data."
else
  rm -rf "$APP_DIR"
  echo "Removed the Headless Watch service, binaries, and local data."
fi
