#!/usr/bin/env bash
# build.sh — Build the active GPUI shell from the repo root.
#
# Usage:
#   ./build.sh                        # Release build
#   BUILD_TYPE=Debug ./build.sh       # Debug build
#   BUILD_DIR=target-root ./build.sh  # Override Cargo target dir
#   PACKAGE_FORMATS=deb,appimage ./build.sh
#   PACKAGE_FORMATS=app MACOS_SIGN=1 MACOS_SIGN_IDENTITY="Developer ID Application: ..." ./build.sh

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
UI_CRATE="${PIER_UI_CRATE:-pier-ui-gpui}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-}"
PACKAGE_FORMATS="${PACKAGE_FORMATS:-}"
HOST_OS="$(uname -s)"

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required command not found: $1" >&2
        exit 1
    fi
}

ensure_ui_dir() {
    if [ ! -d "$ROOT_DIR/$UI_CRATE" ]; then
        echo "ERROR: active GPUI shell crate not found at $ROOT_DIR/$UI_CRATE" >&2
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

normalized_formats() {
    printf '%s' "$PACKAGE_FORMATS" | tr '[:upper:]' '[:lower:]' | tr '; ' ',,'
}

format_requested() {
    local wanted=",$(normalized_formats),"
    case "$wanted" in
        *,all,*|*,"$1",*)
            return 0
            ;;
        *)
            return 1
            ;;
    esac
}

target_root_dir() {
    if [ -n "${CARGO_TARGET_DIR:-}" ]; then
        printf '%s\n' "$CARGO_TARGET_DIR"
    else
        printf '%s\n' "$ROOT_DIR/target"
    fi
}

case "$BUILD_TYPE" in
    Debug|Release) ;;
    *)
        echo "ERROR: BUILD_TYPE must be Debug or Release (got: $BUILD_TYPE)" >&2
        exit 1
        ;;
esac

need_cmd cargo
ensure_ui_dir

if [ -n "$BUILD_DIR" ]; then
    case "$BUILD_DIR" in
        /*|[A-Za-z]:/*|[A-Za-z]:\\*)
            export CARGO_TARGET_DIR="$BUILD_DIR"
            ;;
        *)
            export CARGO_TARGET_DIR="$ROOT_DIR/$BUILD_DIR"
            ;;
    esac
    echo "==> Using Cargo target dir: $CARGO_TARGET_DIR"
fi

CARGO_CMD=(cargo build -p "$UI_CRATE")
if [ "$BUILD_TYPE" = "Release" ]; then
    CARGO_CMD+=(--release)
fi

echo "==> Building Pier-X GPUI shell ($BUILD_TYPE)"
"${CARGO_CMD[@]}"

echo "[OK] Build complete: $(target_root_dir)"

if [ -z "$PACKAGE_FORMATS" ] && \
   ! is_truthy "${MACOS_SIGN:-0}" && \
   ! is_truthy "${MACOS_NOTARIZE:-0}" && \
   [ -z "${MACOS_SIGN_IDENTITY:-}" ]; then
    exit 0
fi

case "$HOST_OS" in
    Linux)
        if format_requested "deb" || format_requested "appimage"; then
            "$ROOT_DIR/scripts/package-linux.sh" --skip-build
        else
            echo "ERROR: unsupported Linux PACKAGE_FORMATS=$PACKAGE_FORMATS (supported: deb, appimage, all)" >&2
            exit 1
        fi
        ;;
    Darwin)
        if format_requested "app" || format_requested "all" || \
           is_truthy "${MACOS_SIGN:-0}" || \
           is_truthy "${MACOS_NOTARIZE:-0}" || \
           [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
            "$ROOT_DIR/scripts/bundle-macos.sh" --skip-build
        else
            echo "ERROR: unsupported macOS PACKAGE_FORMATS=$PACKAGE_FORMATS (supported: app, all)" >&2
            exit 1
        fi
        ;;
    *)
        echo "ERROR: packaging is not implemented for host OS: $HOST_OS" >&2
        exit 1
        ;;
esac
