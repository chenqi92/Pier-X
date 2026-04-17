#!/usr/bin/env bash
# bundle-macos.sh — wrap the cargo binary in a `.app` so macOS shows the
# Pier-X icon (instead of the parent terminal's) in Dock / Cmd-Tab / window
# titlebar.
#
# Usage:
#   ./scripts/bundle-macos.sh                  # debug build → target/debug/Pier-X.app
#   BUILD_TYPE=Release ./scripts/bundle-macos.sh
#   ./scripts/bundle-macos.sh --open           # build + bundle + open the .app

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_TYPE="${BUILD_TYPE:-Debug}"
DO_OPEN=0
for arg in "$@"; do
    case "$arg" in
        --open) DO_OPEN=1 ;;
    esac
done

case "$BUILD_TYPE" in
    Debug)   PROFILE_DIR="debug";   CARGO_FLAGS=() ;;
    Release) PROFILE_DIR="release"; CARGO_FLAGS=(--release) ;;
    *)
        echo "ERROR: BUILD_TYPE must be Debug or Release (got: $BUILD_TYPE)" >&2
        exit 1
        ;;
esac

BIN_NAME="pier-ui-gpui"
APP_NAME="Pier-X"
BUNDLE_ID="com.pier-x.desktop"

TARGET_BIN="$ROOT_DIR/target/$PROFILE_DIR/$BIN_NAME"
APP_DIR="$ROOT_DIR/target/$PROFILE_DIR/$APP_NAME.app"
ICON_SRC="$ROOT_DIR/pier-ui-gpui/assets/app-icons/icon.icns"

# ── Build ──────────────────────────────────────────────────────────────
echo "==> Building ($BUILD_TYPE)…"
( cd "$ROOT_DIR" && cargo build -p "$BIN_NAME" "${CARGO_FLAGS[@]}" )

if [ ! -f "$TARGET_BIN" ]; then
    echo "ERROR: binary not found at $TARGET_BIN" >&2
    exit 1
fi
if [ ! -f "$ICON_SRC" ]; then
    echo "ERROR: icon not found at $ICON_SRC" >&2
    exit 1
fi

# ── Bundle ─────────────────────────────────────────────────────────────
echo "==> Bundling → $APP_DIR"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy the binary in. We use `cp` (not symlink) so the .app is portable —
# a user can move it to /Applications without the link breaking.
cp "$TARGET_BIN" "$APP_DIR/Contents/MacOS/$APP_NAME"
chmod +x "$APP_DIR/Contents/MacOS/$APP_NAME"

cp "$ICON_SRC" "$APP_DIR/Contents/Resources/AppIcon.icns"

# Minimal Info.plist — covers what macOS needs to associate the icon and
# render the window without "Open with…" prompts.
PIER_VERSION="$(grep '^version' "$ROOT_DIR/pier-ui-gpui/Cargo.toml" | head -1 | cut -d'"' -f2)"
cat > "$APP_DIR/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>$APP_NAME</string>
    <key>CFBundleDisplayName</key>
    <string>$APP_NAME</string>
    <key>CFBundleExecutable</key>
    <string>$APP_NAME</string>
    <key>CFBundleIdentifier</key>
    <string>$BUNDLE_ID</string>
    <key>CFBundleVersion</key>
    <string>$PIER_VERSION</string>
    <key>CFBundleShortVersionString</key>
    <string>$PIER_VERSION</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
    <key>NSSupportsAutomaticGraphicsSwitching</key>
    <true/>
</dict>
</plist>
EOF

# Touch the bundle so Launch Services rebuilds the icon cache on next open.
touch "$APP_DIR"

echo "[OK] Bundle ready: $APP_DIR"

if [ "$DO_OPEN" -eq 1 ]; then
    echo "==> Opening…"
    open "$APP_DIR"
fi
