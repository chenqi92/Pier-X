#!/usr/bin/env bash
# create-dmg.sh — Build a polished .dmg installer for Pier-X.
#
# Prerequisites:
#   brew install create-dmg   (or npm install -g create-dmg)
#
# Usage:
#   ./packaging/create-dmg.sh build/pier-ui-qt/pier-x.app [0.1.0]
#
# The script expects macdeployqt to have already been run on the
# .app bundle. It produces Pier-X-<version>.dmg in the current dir.

set -euo pipefail

APP_BUNDLE="${1:?Usage: create-dmg.sh <path-to-pier-x.app> [version]}"
VERSION="${2:-$(cat VERSION | tr -d '[:space:]')}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

if ! command -v create-dmg &>/dev/null; then
    echo "ERROR: create-dmg not found. Install with: brew install create-dmg" >&2
    exit 1
fi

DMG_NAME="Pier-X-${VERSION}.dmg"
VOLUME_NAME="Pier-X ${VERSION}"

# create-dmg does everything: copies the .app, creates the
# Applications symlink, sets background, icon layout, and
# window size in a single invocation.
create-dmg \
    --volname "$VOLUME_NAME" \
    --background "$SCRIPT_DIR/dmg-background.png" \
    --window-pos 200 120 \
    --window-size 600 400 \
    --icon "pier-x.app" 150 250 \
    --app-drop-link 450 250 \
    --icon-size 80 \
    --text-size 14 \
    --hide-extension "pier-x.app" \
    --no-internet-enable \
    "$DMG_NAME" \
    "$APP_BUNDLE"

echo "[OK] Created $DMG_NAME"
