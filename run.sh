#!/usr/bin/env bash
# run.sh — Launch the active GPUI shell from the repo root.
#
# Usage:
#   ./run.sh                        # Debug/dev shell
#   BUILD_TYPE=Release ./run.sh     # Run the GPUI shell in release mode
#   BUILD_DIR=target-root ./run.sh  # Override Cargo target dir

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
UI_CRATE="${PIER_UI_CRATE:-pier-ui-gpui}"
BUILD_TYPE="${BUILD_TYPE:-Debug}"
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
    export CARGO_TARGET_DIR="$ROOT_DIR/$BUILD_DIR"
    echo "==> Using Cargo target dir: $CARGO_TARGET_DIR"
    echo "==> Using Cargo target dir: $CARGO_TARGET_DIR"
fi

CARGO_CMD=(cargo run -p "$UI_CRATE")
if [ "$BUILD_TYPE" = "Release" ]; then
    CARGO_CMD+=(--release)
fi
if [ "$#" -gt 0 ]; then
    CARGO_CMD+=(--)
    CARGO_CMD+=("$@")
fi

echo "==> Launching Pier-X GPUI shell ($BUILD_TYPE)"
exec "${CARGO_CMD[@]}"
