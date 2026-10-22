#!/usr/bin/env bash
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="$(grep -m1 '^version' "$HERE/backend/Cargo.toml" | cut -d'"' -f2)"
ARCH="$(uname -m)"
NAME="headless-watch-${VERSION}-linux-${ARCH}"
DIST="$HERE/dist"
STAGE="$DIST/$NAME"

if [ ! -f "$HERE/frontend/dist/index.html" ]; then
  pnpm --dir "$HERE/frontend" install --frozen-lockfile
  pnpm --dir "$HERE/frontend" build
fi
cargo build --release --manifest-path "$HERE/backend/Cargo.toml"

rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/web" "$STAGE/docs"
install -m 0755 "$HERE/backend/target/release/headless-watch" "$STAGE/bin/headless-watch"
cp -a "$HERE/frontend/dist/." "$STAGE/web/"
cp -a "$HERE/docs/." "$STAGE/docs/"
install -m 0644 "$HERE/README.md" "$STAGE/README.md"
install -m 0644 "$HERE/LICENSE" "$STAGE/LICENSE"
install -m 0755 "$HERE/install.sh" "$STAGE/install.sh"
install -m 0755 "$HERE/uninstall.sh" "$STAGE/uninstall.sh"
find "$STAGE" -type d -name __pycache__ -prune -exec rm -rf {} + 2>/dev/null || true

tar -C "$DIST" -czf "$DIST/$NAME.tar.gz" "$NAME"
sha256sum "$DIST/$NAME.tar.gz" > "$DIST/$NAME.tar.gz.sha256"
echo "Created $DIST/$NAME.tar.gz"
cat "$DIST/$NAME.tar.gz.sha256"
