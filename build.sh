#!/usr/bin/env bash
# build.sh — Build the active Tauri shell from the repo root.
#
# Usage:
#   ./build.sh                        # Release bundle build
#   BUILD_TYPE=Debug ./build.sh       # Debug build
#   BUILD_DIR=target-root ./build.sh  # Override Cargo target dir
#   NO_BUNDLE=1 ./build.sh            # Compile without generating installers

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"
UI_DIR="${PIER_UI_DIR:-$ROOT_DIR/pier-ui-tauri}"
BUILD_TYPE="${BUILD_TYPE:-Release}"
BUILD_DIR="${BUILD_DIR:-}"
NO_BUNDLE="${NO_BUNDLE:-0}"

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "ERROR: required command not found: $1" >&2
        exit 1
    fi
}

ensure_ui_dir() {
    if [ ! -d "$UI_DIR" ]; then
        echo "ERROR: active Tauri shell not found at $UI_DIR" >&2
        exit 1
    fi
}

ensure_node_modules() {
    local lock_marker="node_modules/.package-lock.json"
    if [ ! -d node_modules ] || [ package-lock.json -nt "$lock_marker" ]; then
        echo "==> Installing frontend dependencies"
        npm ci
    fi
}

case "$BUILD_TYPE" in
    Debug|Release) ;;
    *)
        echo "ERROR: BUILD_TYPE must be Debug or Release (got: $BUILD_TYPE)" >&2
        exit 1
        ;;
esac

need_cmd node
need_cmd npm
need_cmd cargo
ensure_ui_dir

if [ -n "$BUILD_DIR" ]; then
    export CARGO_TARGET_DIR="$ROOT_DIR/$BUILD_DIR"
    echo "==> Using Cargo target dir: $CARGO_TARGET_DIR"
fi

cd "$UI_DIR"
ensure_node_modules

TAURI_CMD=(npm run tauri -- build --ci)
if [ "$BUILD_TYPE" = "Debug" ]; then
    TAURI_CMD+=(--debug)
fi
if [ "$NO_BUNDLE" = "1" ]; then
    TAURI_CMD+=(--no-bundle)
fi

echo "==> Building Pier-X Tauri shell ($BUILD_TYPE)"
"${TAURI_CMD[@]}"

if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    echo "[OK] Build complete: $CARGO_TARGET_DIR"
else
    echo "[OK] Build complete: $UI_DIR/src-tauri/target"
fi
