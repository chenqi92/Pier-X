#!/usr/bin/env bash
# build.sh — Build the active GPUI shell from the repo root.
#
# Usage:
#   ./build.sh                        # Release build
#   BUILD_TYPE=Debug ./build.sh       # Debug build
#   BUILD_DIR=target-root ./build.sh  # Override Cargo target dir

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
UI_CRATE="${PIER_UI_CRATE:-pier-ui-gpui}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-}"

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

if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    echo "[OK] Build complete: $CARGO_TARGET_DIR"
else
    echo "[OK] Build complete: $ROOT_DIR/target"
fi
