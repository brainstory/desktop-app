#!/usr/bin/env bash
# Build a Brainstory DMG without Finder automation (tauri's own dmg bundler
# needs AppleScript control of Finder, which fails in CI/SSH/headless shells).
# Usage: scripts/make-dmg.sh   (run after `pnpm tauri:build`)
set -euo pipefail

cd "$(dirname "$0")/.."

APP="src-tauri/target/release/bundle/macos/Brainstory.app"
OUT_DIR="src-tauri/target/release/bundle/dmg"
VERSION=$(python3 -c "import json;print(json.load(open('src-tauri/tauri.conf.json'))['version'])")

[ -d "$APP" ] || { echo "Brainstory.app not found - run 'pnpm tauri:build' first" >&2; exit 1; }
mkdir -p "$OUT_DIR"

STAGING=$(mktemp -d)
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP" "$STAGING/"
ln -sf /Applications "$STAGING/Applications"

OUT="$OUT_DIR/Brainstory_${VERSION}_aarch64.dmg"
rm -f "$OUT"
hdiutil create -volname "Brainstory" -srcfolder "$STAGING" -ov -format UDZO "$OUT"
echo "Created $OUT"
