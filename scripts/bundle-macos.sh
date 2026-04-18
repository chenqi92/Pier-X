#!/usr/bin/env bash
# bundle-macos.sh — wrap the cargo binary in a `.app` so macOS shows the
# Pier-X icon (instead of the parent terminal's) in Dock / Cmd-Tab / window
# titlebar.
#
# Usage:
#   ./scripts/bundle-macos.sh                  # debug build → target/debug/Pier-X.app
#   BUILD_TYPE=Release ./scripts/bundle-macos.sh
#   ./scripts/bundle-macos.sh --open           # build + bundle + open the .app
#   BUILD_TYPE=Release MACOS_SIGN=1 MACOS_SIGN_IDENTITY="Developer ID Application: ..." ./scripts/bundle-macos.sh
#   BUILD_TYPE=Release MACOS_NOTARIZE=1 MACOS_NOTARYTOOL_PROFILE=pier-x ./scripts/bundle-macos.sh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
BUILD_TYPE="${BUILD_TYPE:-Debug}"
DO_OPEN=0
SKIP_BUILD=0
for arg in "$@"; do
    case "$arg" in
        --open) DO_OPEN=1 ;;
        --skip-build) SKIP_BUILD=1 ;;
        *)
            echo "ERROR: unknown argument: $arg" >&2
            exit 1
            ;;
    esac
done

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required command not found: $1" >&2
        exit 1
    fi
}

is_truthy() {
    case "${1:-}" in
        1|true|TRUE|yes|YES|on|ON)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

resolve_target_root() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        printf '%s\n' "$CARGO_TARGET_DIR"
    else
        printf '%s\n' "$ROOT_DIR/target"
    fi
}

resolve_package_root() {
    local requested="${PACKAGE_OUTPUT_DIR:-}"
    if [ -z "$requested" ]; then
        printf '%s\n' "$TARGET_ROOT/$PROFILE_DIR/packages"
        return 0
    fi

    case "$requested" in
        /*)
            printf '%s\n' "$requested"
            ;;
        *)
            printf '%s\n' "$ROOT_DIR/$requested"
            ;;
    esac
}

resolve_version() {
    sed -n 's/^version *= *"\(.*\)"$/\1/p' "$ROOT_DIR/pier-ui-gpui/Cargo.toml" | head -n 1
}

build_binary() {
    need_cmd cargo
    echo "==> Building ($BUILD_TYPE)"
    if [ -n "$CARGO_PROFILE_FLAG" ]; then
        ( cd "$ROOT_DIR" && cargo build -p "$BIN_NAME" "$CARGO_PROFILE_FLAG" )
    else
        ( cd "$ROOT_DIR" && cargo build -p "$BIN_NAME" )
    fi
}

sign_bundle() {
    need_cmd codesign

    if [ -z "${MACOS_SIGN_IDENTITY:-}" ]; then
        echo "ERROR: MACOS_SIGN_IDENTITY is required when MACOS_SIGN=1 or MACOS_NOTARIZE=1" >&2
        exit 1
    fi

    echo "==> Signing .app with codesign"
    codesign_args=(
        codesign
        --force
        --sign "$MACOS_SIGN_IDENTITY"
        --timestamp
        --options runtime
    )
    if [ -n "${MACOS_ENTITLEMENTS:-}" ]; then
        codesign_args+=(--entitlements "$MACOS_ENTITLEMENTS")
    fi
    codesign_args+=("$APP_DIR")
    "${codesign_args[@]}"

    codesign --verify --verbose=2 "$APP_DIR"
    echo "[OK] Signed bundle: $APP_DIR"
}

notarize_bundle() {
    need_cmd xcrun
    mkdir -p "$MACOS_PACKAGE_ROOT"

    ZIP_PATH="$MACOS_PACKAGE_ROOT/Pier-X_${PIER_VERSION}_macos.zip"
    echo "==> Creating notarization archive -> $ZIP_PATH"
    rm -f "$ZIP_PATH"
    ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" "$ZIP_PATH"

    if [ -n "${MACOS_NOTARYTOOL_PROFILE:-}" ]; then
        echo "==> Submitting bundle to Apple notary service via keychain profile"
        xcrun notarytool submit "$ZIP_PATH" --keychain-profile "$MACOS_NOTARYTOOL_PROFILE" --wait
    elif [ -n "${MACOS_NOTARY_APPLE_ID:-}" ] && [ -n "${MACOS_NOTARY_PASSWORD:-}" ] && [ -n "${MACOS_NOTARY_TEAM_ID:-}" ]; then
        echo "==> Submitting bundle to Apple notary service via Apple ID credentials"
        xcrun notarytool submit \
            "$ZIP_PATH" \
            --apple-id "$MACOS_NOTARY_APPLE_ID" \
            --password "$MACOS_NOTARY_PASSWORD" \
            --team-id "$MACOS_NOTARY_TEAM_ID" \
            --wait
    else
        echo "ERROR: notarization requires MACOS_NOTARYTOOL_PROFILE or MACOS_NOTARY_APPLE_ID + MACOS_NOTARY_PASSWORD + MACOS_NOTARY_TEAM_ID" >&2
        exit 1
    fi

    echo "==> Stapling notarization ticket"
    xcrun stapler staple "$APP_DIR"
    xcrun stapler validate "$APP_DIR"
    echo "[OK] Notarized bundle: $APP_DIR"
}

case "$BUILD_TYPE" in
    Debug)   PROFILE_DIR="debug";   CARGO_PROFILE_FLAG="" ;;
    Release) PROFILE_DIR="release"; CARGO_PROFILE_FLAG="--release" ;;
    *)
        echo "ERROR: BUILD_TYPE must be Debug or Release (got: $BUILD_TYPE)" >&2
        exit 1
        ;;
esac

BIN_NAME="pier-ui-gpui"
APP_NAME="Pier-X"
BUNDLE_ID="com.pier-x.desktop"

if [ -n "${BUILD_DIR:-}" ] && [ -z "${CARGO_TARGET_DIR:-}" ]; then
    case "$BUILD_DIR" in
        /*)
            export CARGO_TARGET_DIR="$BUILD_DIR"
            ;;
        *)
            export CARGO_TARGET_DIR="$ROOT_DIR/$BUILD_DIR"
            ;;
    esac
fi

TARGET_ROOT="$(resolve_target_root)"
TARGET_BIN="$TARGET_ROOT/$PROFILE_DIR/$BIN_NAME"
APP_DIR="$TARGET_ROOT/$PROFILE_DIR/$APP_NAME.app"
ICON_SRC="$ROOT_DIR/pier-ui-gpui/assets/app-icons/icon.icns"
PIER_VERSION="$(resolve_version)"
PACKAGE_ROOT="$(resolve_package_root)"
MACOS_PACKAGE_ROOT="$PACKAGE_ROOT/macos"
MACOS_SIGN_REQUESTED=0
MACOS_NOTARIZE_REQUESTED=0

if is_truthy "${MACOS_SIGN:-0}" || [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    MACOS_SIGN_REQUESTED=1
fi

if is_truthy "${MACOS_NOTARIZE:-0}"; then
    MACOS_SIGN_REQUESTED=1
    MACOS_NOTARIZE_REQUESTED=1
fi

# ── Build ──────────────────────────────────────────────────────────────
if [ "$SKIP_BUILD" -eq 0 ]; then
    build_binary
fi

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

if [ "$MACOS_SIGN_REQUESTED" -eq 1 ]; then
    sign_bundle
fi

if [ "$MACOS_NOTARIZE_REQUESTED" -eq 1 ]; then
    notarize_bundle
fi

if [ "$DO_OPEN" -eq 1 ]; then
    echo "==> Opening…"
    open "$APP_DIR"
fi
